.pragma library

// Missing or malformed readings stay unavailable, never a misleading zero.
function parse(source) {
  var sample = { cpu: null, memory: null, disk: null }
  var lines = String(source).trim().split("\n")
  for (var i = 0; i < lines.length; i++) {
    var fields = lines[i].trim().split(/\s+/)
    var kind = fields.shift()
    if (!fields.every(function(s) { return /^-?[0-9]+$/.test(s) && isFinite(Number(s)) })) continue
    var n = fields.map(Number)
    if (kind === "cpu" && n.length === 2 && n[0] >= 0 && n[1] >= 0 && n[1] <= n[0])
      sample.cpu = { total: n[0], idle: n[1] }
    if (kind === "memory" && n.length === 2 && n[0] > 0 && n[1] >= 0 && n[1] <= n[0])
      sample.memory = { total: n[0], used: n[0] - n[1], available: n[1], percent: 100 * (n[0] - n[1]) / n[0] }
    // Available space can be negative when reserved filesystem space is used.
    if (kind === "disk" && n.length === 4 && n[0] > 0 && n[1] >= 0 && n[3] >= 0)
      sample.disk = { total: n[0], used: n[1], available: n[2], percent: n[3] }
  }
  return sample
}

function cpuPercent(previous, current) {
  if (!previous || !current) return null
  var total = current.total - previous.total
  var idle = current.idle - previous.idle
  if (total <= 0 || idle < 0 || idle > total) return null
  return 100 * (total - idle) / total
}

// Inputs come from /proc/meminfo and df -k, both in KiB.
function size(kib) {
  var units = ["KiB", "MiB", "GiB", "TiB", "PiB"]
  var value = kib, unit = 0
  while (Math.abs(value) >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return { value: value, unit: units[unit] }
}
