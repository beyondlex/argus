pub mod audit;
pub mod categories;
#[allow(clippy::module_inception)]
pub mod cleaner;
pub mod purge;
pub mod safety;
#[cfg(feature = "shell-cmds")]
pub mod shell_cmd;
pub mod uninstaller;
