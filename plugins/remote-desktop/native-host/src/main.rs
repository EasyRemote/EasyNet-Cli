// EasyNet RemoteApp native host
// =============================
//
// Private executable dependency of the native-static Remote Desktop plugin.
// It accepts no command-line interface and owns no Runtime, identity,
// authority, session, resource, or receipt state.

fn main() -> anyhow::Result<()> {
    if std::env::args_os().len() != 1 {
        anyhow::bail!("easynet-remoteapp-native-host accepts no arguments")
    }
    easynet_remoteapp_native_host::run()
}
