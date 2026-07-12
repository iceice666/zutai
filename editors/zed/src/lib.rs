use zed_extension_api as zed;

struct ZutaiExtension;

impl zed::Extension for ZutaiExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let command = worktree
            .which("zutai-cli")
            .ok_or_else(|| "could not find `zutai-cli` on the worktree PATH".to_string())?;
        Ok(zed::Command::new(command).arg("lsp"))
    }
}

zed::register_extension!(ZutaiExtension);
