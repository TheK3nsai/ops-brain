export function bindLiveLifecycle(mcp, live) {
  let started = false
  mcp.oninitialized = () => {
    if (started) return
    started = true
    live.start()
  }
  return () => live.stop()
}
