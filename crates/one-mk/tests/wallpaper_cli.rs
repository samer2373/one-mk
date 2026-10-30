use std::process::Command;

#[test]
fn wallpaper_dispatch_never_initializes_owner_configuration() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wall-import-lab");
    std::fs::create_dir_all(&root).unwrap();
    let private = tempfile::tempdir_in(root).unwrap();
    for arguments in [["wallpaper", "doctor", "--runtime", "missing", "--json"].as_slice(), ["wallpaper", "import", "missing", "--json"].as_slice()] {
        let result = Command::new(env!("CARGO_BIN_EXE_one-mk")).args(arguments).env_clear()
            .env("HOME", private.path()).env("XDG_CONFIG_HOME", private.path()).env("XDG_RUNTIME_DIR", private.path())
            .output().unwrap();
        assert!(!result.status.success());
        let value: serde_json::Value = serde_json::from_slice(&result.stdout).expect("finite JSON helper/refusal response");
        assert!(value.get("error").is_some() || value.get("available") == Some(&serde_json::Value::Bool(false)));
        assert_eq!(std::fs::read_dir(private.path()).unwrap().count(), 0, "CLI must dispatch before config/identity initialization");
    }
}
