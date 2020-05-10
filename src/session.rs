#[derive(Debug, Clone)]
pub struct SessionId(String);

impl<T> From<T> for SessionId
    where
        T: Into<String>,
{
    fn from(value: T) -> Self {
        SessionId(value.into())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl SessionId {
    pub fn new() -> Self {
        Self::default()
    }
}

struct Session {
    id: SessionId,
}

impl Session {
    pub fn new(db_file: String) -> Self {
        Self {
            id: SessionId::new(),
        }
    }
}
