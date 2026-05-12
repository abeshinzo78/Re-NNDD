//! Persistence layer. Owns the SQLite database, migrations, and CRUD over
//! library entities (videos, tags, comments, NG rules, …).

pub mod db;
pub mod query;
pub mod queue;
pub mod schema;
pub mod settings;
pub mod videos;
