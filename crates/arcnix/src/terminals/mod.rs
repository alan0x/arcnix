mod local_terminal;
pub(crate) use local_terminal::check_support as check_local_terminal;
pub(crate) mod terminal;
pub(crate) mod terminal_menu;
pub(crate) mod terminal_worker;
#[cfg(test)]
pub(crate) mod terminal_worker_tests;
