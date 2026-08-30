fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut arguments = std::env::args_os().skip(1);
        match arguments.next().as_deref() {
            Some(argument)
                if argument
                    == std::ffi::OsStr::new(
                        easynet_remoteapp_native_protocol::macos_launch_services::ARG,
                    ) =>
            {
                let socket = arguments
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing LaunchServices bootstrap socket"))?;
                anyhow::ensure!(arguments.next().is_none(), "unexpected bootstrap argument");
                return easynet_remoteapp_media_host::run_launch_services_bootstrap(socket);
            }
            Some(argument)
                if argument == std::ffi::OsStr::new("--request-screen-capture-permission") =>
            {
                return easynet_remoteapp_media_host::run_screen_capture_permission_application();
            }
            Some(_) => anyhow::bail!("unsupported media-host application argument"),
            None => {}
        }
    }
    easynet_remoteapp_media_host::run()
}
