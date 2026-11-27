func startService() {
  fatalError("start the app's main service")
}

@MainActor
func runUILoop(_ ui: AppUI) {
  fatalError("run the UI loop of the OS on this thread")
  // The loop ends when the app UI is told that the app can quit.
}

func stopService() {
  fatalError("stop the app's main service")
}

startService()
let ui = AppUI()
ui.setUp()
runUILoop(ui)
stopService()
