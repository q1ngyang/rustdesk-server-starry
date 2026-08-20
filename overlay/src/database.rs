use hbb_common::ResultType;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio_rusqlite::{params, rusqlite, Connection};

#[derive(Clone)]
pub struct Database {
    connections: Arc<Vec<Connection>>,
    next: Arc<AtomicUsize>,
}

#[derive(Default)]
pub struct Peer {
    pub guid: Vec<u8>,
    pub id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    pub user: Option<Vec<u8>>,
    pub info: String,
    pub status: Option<i64>,
}

impl Database {
    pub async fn new(url: &str) -> ResultType<Database> {
        let connection_count = std::env::var("MAX_DATABASE_CONNECTIONS")
            .unwrap_or_else(|_| "1".to_owned())
            .parse::<usize>()
            .unwrap_or(1)
            .max(1);
        hbb_common::log::debug!("MAX_DATABASE_CONNECTIONS={connection_count}");
        let mut connections = Vec::with_capacity(connection_count);
        for _ in 0..connection_count {
            connections.push(Connection::open(url).await?);
        }
        let database = Database {
            connections: Arc::new(connections),
            next: Arc::new(AtomicUsize::new(0)),
        };
        database.create_tables().await?;
        Ok(database)
    }

    fn connection(&self) -> &Connection {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        &self.connections[index]
    }

    async fn create_tables(&self) -> ResultType<()> {
        self.connection()
            .call(|connection| {
                connection.execute_batch(
                    "
                    create table if not exists peer (
                        guid blob primary key not null,
                        id varchar(100) not null,
                        uuid blob not null,
                        pk blob not null,
                        created_at datetime not null default(current_timestamp),
                        user blob,
                        status tinyint,
                        note varchar(300),
                        info text not null
                    ) without rowid;
                    create unique index if not exists index_peer_id on peer (id);
                    create index if not exists index_peer_user on peer (user);
                    create index if not exists index_peer_created_at on peer (created_at);
                    create index if not exists index_peer_status on peer (status);
                    ",
                )?;
                Ok::<_, rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_peer(&self, id: &str) -> ResultType<Option<Peer>> {
        let id = id.to_owned();
        Ok(self
            .connection()
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "select guid, id, uuid, pk, user, status, info from peer where id = ?1",
                )?;
                let mut rows = statement.query(params![id])?;
                let peer = match rows.next()? {
                    Some(row) => Some(Peer {
                        guid: row.get(0)?,
                        id: row.get(1)?,
                        uuid: row.get(2)?,
                        pk: row.get(3)?,
                        user: row.get(4)?,
                        status: row.get(5)?,
                        info: row.get(6)?,
                    }),
                    None => None,
                };
                Ok::<_, rusqlite::Error>(peer)
            })
            .await?)
    }

    pub async fn insert_peer(
        &self,
        id: &str,
        uuid: &[u8],
        pk: &[u8],
        info: &str,
    ) -> ResultType<Vec<u8>> {
        let guid = uuid::Uuid::new_v4().as_bytes().to_vec();
        let id = id.to_owned();
        let uuid = uuid.to_vec();
        let pk = pk.to_vec();
        let info = info.to_owned();
        Ok(self
            .connection()
            .call(move |connection| {
                connection.execute(
                    "insert into peer(guid, id, uuid, pk, info) values(?1, ?2, ?3, ?4, ?5)",
                    params![&guid, id, uuid, pk, info],
                )?;
                Ok::<_, rusqlite::Error>(guid)
            })
            .await?)
    }

    pub async fn update_pk(
        &self,
        guid: &Vec<u8>,
        id: &str,
        pk: &[u8],
        info: &str,
    ) -> ResultType<()> {
        let guid = guid.clone();
        let id = id.to_owned();
        let pk = pk.to_vec();
        let info = info.to_owned();
        self.connection()
            .call(move |connection| {
                connection.execute(
                    "update peer set id=?1, pk=?2, info=?3 where guid=?4",
                    params![id, pk, info, guid],
                )?;
                Ok::<_, rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use hbb_common::tokio;
    #[test]
    fn test_insert() {
        insert();
    }

    #[tokio::main(flavor = "multi_thread")]
    async fn insert() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("test.sqlite3");
        let db = super::Database::new(database_path.to_str().unwrap())
            .await
            .unwrap();
        let mut jobs = vec![];
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                let empty_vec = Vec::new();
                cloned
                    .insert_peer(&id, &empty_vec, &empty_vec, "")
                    .await
                    .unwrap();
            });
            jobs.push(a);
        }
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                cloned.get_peer(&id).await.unwrap();
            });
            jobs.push(a);
        }
        for result in hbb_common::futures::future::join_all(jobs).await {
            result.unwrap();
        }
    }
}
