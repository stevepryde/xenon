pub struct SessionPool {
    sessions: RwLock<HashMap<XenonSessionId, Session>>,
}
