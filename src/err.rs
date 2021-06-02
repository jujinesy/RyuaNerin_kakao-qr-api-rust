#[derive(Debug)]
pub enum HandlerError {
    BadStatusCode(u16),
    CannotFindToken,
    HyperError(hyper::Error),
    ReqwestError(reqwest::Error),
    FailureError(failure::Error),
}

impl std::fmt::Display for HandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            HandlerError::BadStatusCode(status_code) => write!(f, "qr_check_in returned: {}", status_code),
            HandlerError::CannotFindToken => write!(f, "Cannot find token"),
            HandlerError::HyperError(ref e) => e.fmt(f),
            HandlerError::ReqwestError(ref e) => e.fmt(f),
            HandlerError::FailureError(ref e) => e.fmt(f),
        }
    }
}

impl From<hyper::Error> for HandlerError {
    fn from(err: hyper::Error) -> HandlerError {
        HandlerError::HyperError(err)
    }
}

impl From<failure::Error> for HandlerError {
    fn from(err: failure::Error) -> HandlerError {
        HandlerError::FailureError(err)
    }
}

impl From<reqwest::Error> for HandlerError {
    fn from(err: reqwest::Error) -> HandlerError {
        HandlerError::ReqwestError(err)
    }
}
