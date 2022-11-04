use crate::compose;
use rocket::serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize, Copy, Clone)]
pub enum Status {
    Fetching,
    Reading,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Context {
    pub status: Status,
    pub run: compose::Run,
    pub id: Uuid,
}
