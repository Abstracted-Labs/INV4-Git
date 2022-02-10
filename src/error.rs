use std::{
    error::Error,
    fmt::{Debug, Display, Formatter, Result},
};

#[derive(Debug)]
pub struct ErrorWrap<T: Debug>(pub T);

impl<T: Debug> Display for ErrorWrap<T> {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "{:?}", &self)
    }
}

impl<T: Debug> Error for ErrorWrap<T> {}
