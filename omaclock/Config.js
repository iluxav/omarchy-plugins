.pragma library

// Reads omaclock.yaml. This is a reader for the small slice of YAML the file
// uses, not a general one: top-level `key: value` lines, and a `zones:` list
// whose items are either
//
//   - label: IL            block form: "- " starts an item, the lines
//     tz: Asia/Jerusalem   indented under it continue it
//
//   - { label: IL, tz: Asia/Jerusalem }     flow form, on one line
//
// Values are plain, "double-quoted" or 'single-quoted'. `#` starts a comment.
// Anything else (anchors, nested lists, multi-line strings) is an error with a
// line number, rather than something silently misread.

var TOP_KEYS = ["separator", "format", "tooltipFormat", "zones"]
var ZONE_KEYS = ["label", "tz", "format"]

// What the tz goes through before it can reach a shell command line: IANA
// names only. (It is passed as an argument, never pasted into a command; this
// is so a typo says so plainly, instead of quietly showing UTC.)
var TZ_PATTERN = /^[A-Za-z0-9_+\-]+(\/[A-Za-z0-9_+\-]+)*$/

function defaults() {
  return {
    separator: "  ·  ",
    format: "%H:%M",
    tooltipFormat: "%A %d %B %H:%M (%Z)",
    zones: []
  }
}

function fail(lineNo, message) {
  throw new Error("line " + lineNo + ": " + message)
}

// Drop a trailing comment, leaving `#` inside quotes alone. A quote only opens
// a string where a value could start, so the apostrophe in "don't" is text.
function stripComment(line) {
  var quote = ""
  for (var i = 0; i < line.length; i++) {
    var c = line.charAt(i)
    if (quote) {
      if (c === "\\" && quote === "\"") i++
      else if (c === "'" && quote === "'" && line.charAt(i + 1) === "'") i++
      else if (c === quote) quote = ""
    } else if ((c === "\"" || c === "'") && (i === 0 || /[\s:{,\[-]/.test(line.charAt(i - 1)))) {
      quote = c
    } else if (c === "#" && (i === 0 || /\s/.test(line.charAt(i - 1)))) {
      return line.slice(0, i)
    }
  }
  return line
}

function scalar(text, lineNo) {
  var s = text.trim()
  if (s.charAt(0) === "\"") {
    var out = ""
    for (var i = 1; i < s.length; i++) {
      var c = s.charAt(i)
      if (c === "\"") {
        if (s.slice(i + 1).trim() !== "") fail(lineNo, "unexpected text after the closing quote")
        return out
      }
      if (c === "\\" && i + 1 < s.length) {
        var n = s.charAt(++i)
        out += n === "n" ? "\n" : n === "t" ? "\t" : n
      } else out += c
    }
    fail(lineNo, "unclosed \"")
  }
  if (s.charAt(0) === "'") {
    var lit = ""
    for (var j = 1; j < s.length; j++) {
      var d = s.charAt(j)
      if (d === "'") {
        if (s.charAt(j + 1) === "'") { lit += "'"; j++; continue }
        if (s.slice(j + 1).trim() !== "") fail(lineNo, "unexpected text after the closing quote")
        return lit
      }
      lit += d
    }
    fail(lineNo, "unclosed '")
  }
  return s
}

// "key: value" -> [key, valueText]. The colon has to be followed by a space
// or the end, so "%H:%M" is a value, not a key.
function splitPair(text, lineNo) {
  var m = /:(\s|$)/.exec(text)
  if (!m) fail(lineNo, "expected key: value")
  var key = text.slice(0, m.index).trim()
  if (key === "") fail(lineNo, "missing key before the colon")
  return [key, text.slice(m.index + 1).trim()]
}

// Split "a: 1, b: \"x, y\"" on the commas that are not inside quotes.
function splitFlow(inner, lineNo) {
  var parts = [], quote = "", start = 0
  for (var i = 0; i < inner.length; i++) {
    var c = inner.charAt(i)
    if (quote) {
      if (c === "\\" && quote === "\"") i++
      else if (c === "'" && quote === "'" && inner.charAt(i + 1) === "'") i++
      else if (c === quote) quote = ""
    } else if ((c === "\"" || c === "'") && (i === 0 || /[\s:{,]/.test(inner.charAt(i - 1)))) {
      quote = c
    } else if (c === ",") {
      parts.push(inner.slice(start, i))
      start = i + 1
    }
  }
  if (quote) fail(lineNo, "unclosed " + quote)
  parts.push(inner.slice(start))
  return parts.filter(function(p) { return p.trim() !== "" })
}

function setZoneKey(zone, key, valueText, lineNo) {
  if (ZONE_KEYS.indexOf(key) === -1) fail(lineNo, "unknown zone key \"" + key + "\" (use label, tz or format)")
  zone[key] = scalar(valueText, lineNo)
}

function finishZone(zone, lineNo, top) {
  if (!zone.tz) fail(zone.line, "zone has no tz")
  if (!TZ_PATTERN.test(zone.tz)) fail(zone.line, "\"" + zone.tz + "\" is not a time zone name (try Asia/Jerusalem)")
  var label = zone.label
  if (label === undefined) label = zone.tz.split("/").pop().replace(/_/g, " ")
  return {
    label: label,
    tz: zone.tz,
    format: zone.format || top.format,
    tooltipFormat: top.tooltipFormat
  }
}

function parse(source) {
  var cfg = defaults()
  var lines = String(source).replace(/\r/g, "").split("\n")
  var inZones = false
  var zone = null
  var raw = []

  function closeZone() {
    if (zone) raw.push(zone)
    zone = null
  }

  for (var i = 0; i < lines.length; i++) {
    var lineNo = i + 1
    var line = stripComment(lines[i])
    if (line.trim() === "") continue
    if (line.indexOf("\t") !== -1 && /^\s*\t/.test(line)) fail(lineNo, "indent with spaces, not tabs")

    var indent = line.length - line.replace(/^ +/, "").length
    var body = line.trim()

    if (indent === 0) {
      closeZone()
      var top = splitPair(body, lineNo)
      if (TOP_KEYS.indexOf(top[0]) === -1) fail(lineNo, "unknown key \"" + top[0] + "\" (use " + TOP_KEYS.join(", ") + ")")
      if (top[0] === "zones") {
        if (top[1] !== "" && top[1] !== "[]") fail(lineNo, "zones is a list: put each zone on its own \"- \" line")
        inZones = true
      } else {
        inZones = false
        cfg[top[0]] = scalar(top[1], lineNo)
      }
      continue
    }

    if (!inZones) fail(lineNo, "indented line outside of zones")

    if (body.charAt(0) === "-" && (body.length === 1 || /\s/.test(body.charAt(1)))) {
      closeZone()
      zone = { line: lineNo }
      var rest = body.slice(1).trim()
      if (rest === "") continue
      if (rest.charAt(0) === "{") {
        if (rest.charAt(rest.length - 1) !== "}") fail(lineNo, "unclosed {")
        var pairs = splitFlow(rest.slice(1, -1), lineNo)
        for (var p = 0; p < pairs.length; p++) {
          var kv = splitPair(pairs[p].trim(), lineNo)
          setZoneKey(zone, kv[0], kv[1], lineNo)
        }
        closeZone()
      } else {
        var first = splitPair(rest, lineNo)
        setZoneKey(zone, first[0], first[1], lineNo)
      }
      continue
    }

    if (!zone) fail(lineNo, "expected \"- \" to start a zone")
    var pair = splitPair(body, lineNo)
    setZoneKey(zone, pair[0], pair[1], lineNo)
  }
  closeZone()

  cfg.zones = raw.map(function(z) { return finishZone(z, z.line, cfg) })
  return cfg
}
