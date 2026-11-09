(() => { const b = Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge;
 b.setMode("clone", "hide");
 return b.startRenderer(["python3", "/tmp/omk-gs-spike/client.py", "--log", "/tmp/omk-gs-spike/log/fps.log"]); })()
