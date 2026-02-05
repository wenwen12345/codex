/// Update action the CLI should perform after the TUI exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Update via `npm install -g @echoflux537/codex`.
    NpmGlobalLatest,
    /// Update via `pnpm add -g @echoflux537/codex`.
    PnpmGlobalLatest,
    /// Update via `bun install -g @echoflux537/codex`.
    BunGlobalLatest,
}

impl UpdateAction {
    /// Returns the list of command-line arguments for invoking the update.
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        match self {
            UpdateAction::NpmGlobalLatest => ("npm", &["install", "-g", "@echoflux537/codex"]),
            UpdateAction::PnpmGlobalLatest => ("pnpm", &["add", "-g", "@echoflux537/codex"]),
            UpdateAction::BunGlobalLatest => ("bun", &["install", "-g", "@echoflux537/codex"]),
        }
    }

    /// Returns string representation of the command-line arguments for invoking the update.
    pub fn command_str(self) -> String {
        let (command, args) = self.command_args();
        shlex::try_join(std::iter::once(command).chain(args.iter().copied()))
            .unwrap_or_else(|_| format!("{command} {}", args.join(" ")))
    }
}

fn command_is_available(command: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    let command_path = std::path::Path::new(command);
    let has_extension = command_path.extension().is_some();
    let is_bare_command = command_path.components().count() == 1;

    #[cfg(windows)]
    let pathext = std::env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());

    for dir in std::env::split_paths(&path_var) {
        if !is_bare_command {
            if dir.join(command).is_file() {
                return true;
            }
            continue;
        }

        #[cfg(windows)]
        {
            if has_extension && dir.join(command).is_file() {
                return true;
            }
            if !has_extension {
                for ext in pathext
                    .to_string_lossy()
                    .split(';')
                    .filter(|ext| !ext.is_empty())
                {
                    let candidate = format!("{command}{ext}");
                    if dir.join(candidate).is_file() {
                        return true;
                    }
                }
            }
        }

        #[cfg(not(windows))]
        {
            let _ = has_extension;
            if dir.join(command).is_file() {
                return true;
            }
        }
    }

    false
}

#[cfg(not(debug_assertions))]
pub(crate) fn get_update_actions() -> Vec<UpdateAction> {
    let managed_by_npm = std::env::var_os("CODEX_MANAGED_BY_NPM").is_some();
    let managed_by_bun = std::env::var_os("CODEX_MANAGED_BY_BUN").is_some();

    let pnpm_available = command_is_available("pnpm");
    detect_update_actions(managed_by_npm, managed_by_bun, pnpm_available)
}

#[cfg(any(not(debug_assertions), test))]
fn detect_update_actions(
    managed_by_npm: bool,
    managed_by_bun: bool,
    pnpm_available: bool,
) -> Vec<UpdateAction> {
    if managed_by_npm {
        let mut actions = vec![UpdateAction::NpmGlobalLatest];
        if pnpm_available {
            actions.push(UpdateAction::PnpmGlobalLatest);
        }
        actions
    } else if managed_by_bun {
        vec![UpdateAction::BunGlobalLatest]
    } else {
        // Default to npm if no specific manager is detected
        vec![UpdateAction::NpmGlobalLatest]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_update_action_without_env_mutation() {
        // Default to npm when no manager is detected
        assert_eq!(
            detect_update_actions(false, false, false),
            vec![UpdateAction::NpmGlobalLatest]
        );
        // npm managed
        assert_eq!(
            detect_update_actions(true, false, false),
            vec![UpdateAction::NpmGlobalLatest]
        );
        // When CODEX_MANAGED_BY_NPM is set, enable pnpm if available.
        assert_eq!(
            detect_update_actions(true, false, true),
            vec![
                UpdateAction::NpmGlobalLatest,
                UpdateAction::PnpmGlobalLatest
            ]
        );
        // bun managed
        assert_eq!(
            detect_update_actions(false, true, false),
            vec![UpdateAction::BunGlobalLatest]
        );
        // npm takes precedence over bun
        assert_eq!(
            detect_update_actions(true, true, true),
            vec![
                UpdateAction::NpmGlobalLatest,
                UpdateAction::PnpmGlobalLatest
            ]
        );
    }
}
