export function bindLiveLifecycle(mcp, live, { warningMs = 10_000, warn = () => {} } = {}) {
  let started = false
  const previous = mcp.oninitialized
  const warning = setTimeout(() => {
    if (!started) warn('Claude MCP client has not initialized; ops-brain online delivery remains disconnected')
  }, warningMs)
  warning.unref?.()
  const initialized = (...args) => {
    clearTimeout(warning)
    previous?.apply(mcp, args)
    if (started) return
    started = true
    live.start()
  }
  mcp.oninitialized = initialized
  return () => {
    clearTimeout(warning)
    if (mcp.oninitialized === initialized) mcp.oninitialized = previous
    if (started) live.stop()
  }
}
