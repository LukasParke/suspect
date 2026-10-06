-- Neovim headless smoke: does the suspect LSP attach and answer under
-- Neovim's own client?
--
-- Neovim exercises things VS Code does not: vim.lsp's own request
-- framing, its cancellation timing, and a leaner capability set. Driven
-- headless so it runs from a shell with no editor UI.
--
--    nvim --headless -l editors/multiclient/nvim_smoke.lua /path/to/spec
--
-- Prints SMOKE| lines; exits non-zero on failure. The first argument is
-- the directory holding the OpenAPI document to open.

local results = {}
local function report(ok, msg)
  table.insert(results, (ok and 'SMOKE|PASS — ' or 'SMOKE|FAIL — ') .. msg)
  if not ok then
    for _, r in ipairs(results) do print(r) end
    vim.cmd('cq!')
  end
end

local dir = arg[1] or '.'
local spec = vim.fn.glob(dir .. '/openapi.yaml')
if spec == '' then
  report(false, 'no openapi.yaml in ' .. dir)
end

-- Handshake with the real binary.
vim.lsp.set_log_level('WARN')
local client_id = vim.lsp.start({
  name = 'suspect',
  cmd = { os.getenv('SUSPECT_BENCH_EXE') or 'suspect', 'lsp' },
  root_dir = vim.fs.dirname(spec),
})
if not client_id then
  report(false, 'vim.lsp.start returned nil — the server never attached')
end
report(client_id ~= nil, 'server attached as client ' .. tostring(client_id))

vim.cmd('edit ' .. vim.fn.fnameescape(spec))
local buf = vim.api.nvim_get_current_buf()

-- Wait for the buffer to be attached to the client.
local attached = vim.wait(15000, function()
  for _, c in ipairs(vim.lsp.get_clients({ bufnr = buf })) do
    if c.id == client_id then return true end
  end
  return false
end, 100)
report(attached, 'buffer attached to the client')

local client = vim.lsp.get_client_by_id(client_id)
report(client ~= nil and client.server_capabilities.hoverProvider == true,
  'server advertises hover')

-- hover on the document's title line.
local ok, hover = vim.wait(15000, function()
  local done = false
  local got = nil
  client.request('textDocument/hover', {
    textDocument = { uri = vim.uri_from_fname(spec) },
    position = { line = 1, character = 2 },
  }, function(err, result)
    done = true
    got = (err and { err = err }) or result
  end, buf)
  if not vim.wait(10000, function() return done end, 50) then return false end
  return got ~= nil
end, 100)
report(ok, 'hover answered')

-- symbols.
local ok_sym, symbols = vim.wait(15000, function()
  local done, got = false, nil
  client.request('textDocument/documentSymbol', {
    textDocument = { uri = vim.uri_from_fname(spec) },
  }, function(err, result)
    done = true
    got = (err and { err = err }) or result
  end, buf)
  if not vim.wait(10000, function() return done end, 50) then return false end
  return type(got) == 'table' and #got > 0
end, 100)
report(ok_sym, 'documentSymbol returned ' ..
  (type(symbols) == 'table' and tostring(#symbols) or '?') .. ' symbols')

-- diagnostics: the server pushes them.
local pushed = vim.wait(15000, function()
  return #vim.diagnostic.get(buf, { namespace = vim.lsp.diagnostic.get_namespace(client_id) }) > 0
end, 100)
report(pushed, 'diagnostics were pushed to the buffer')

for _, r in ipairs(results) do print(r) end
vim.cmd('qa!')
