use std::collections::HashMap;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

#[allow(dead_code)] // till I use it!
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Deserialize, Serialize)]
#[repr(transparent)]
pub struct Id(uuid::Uuid);

#[allow(dead_code)] // till I use it!
#[derive(Debug, Deserialize, Serialize)]
pub struct Collection {
   pub id: Id,
   pub name: String,
   pub pages: HashMap<Id, crate::page::Id>,
}
