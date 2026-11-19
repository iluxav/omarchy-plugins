-- omanote: SUPER+N opens the Omanote search popup from the Omarchy bar
-- (find a note, start one, or `>` to search inside them). An optional second
-- chord opens the one-line quick-capture box instead.
--
--   om.use("omanote").setup({})
--
-- Options:
--   chord   = "SUPER + N"   open or close the notes popup
--   capture = nil           quick capture, a line into the inbox note
--                           (SUPER + SHIFT + N is Omarchy's Editor; pick a free one)
--
-- Needs the iluxav.omanote shell plugin to be enabled.

local M = {}

local plugin = "iluxav.omanote"

function M.setup(opts)
  opts = opts or {}

  om.hotkey(opts.chord or "SUPER + N", function()
    om.spawn("omarchy-shell shell toggle " .. plugin)
  end, { label = "Omanote: find or start a note" })

  if opts.capture then
    om.hotkey(opts.capture, function()
      om.spawn("omarchy-shell shell summon " .. plugin .. " '{\"mode\":\"capture\"}'")
    end, { label = "Omanote: quick note" })
  end

  return M
end

return M
