use std::fmt::Display;

pub trait ResultExt<T> {
    /// Reports the error on stderr and drops it. Only for failures the user
    /// cannot act on, such as updating a view that was already closed.
    fn log_err(self) -> Option<T>;
}

impl<T, E: Display> ResultExt<T> for Result<T, E> {
    fn log_err(self) -> Option<T> {
        match self {
            Ok(value) => Some(value),
            Err(error) => {
                eprintln!("{error:#}");
                None
            }
        }
    }
}
