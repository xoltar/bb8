//! Redis support for the `bb8` connection pool.
#![deny(missing_docs, missing_debug_implementations)]

pub extern crate bb8;
pub extern crate redis;

extern crate futures;
extern crate tokio;

use futures::{Future, IntoFuture};

use redis::async::Connection;
use redis::{Client, RedisError};

use std::io;
use std::option::Option;

type Result<T> = std::result::Result<T, RedisError>;

/// `RedisPool` is a convenience wrapper around `bb8::Pool` that hides the fact that
/// `RedisConnectionManager` uses an `Option<Connection>` to smooth over the API incompatibility.
#[derive(Debug)]
pub struct RedisPool {
    pool: bb8::Pool<RedisConnectionManager>,
}

impl RedisPool {
    /// Constructs a new `RedisPool`, see the `bb8::Builder` documentation for description of
    /// parameters.
    pub fn new(pool: bb8::Pool<RedisConnectionManager>) -> RedisPool {
        RedisPool { pool }
    }

    /// Run the function with a connection provided by the pool.
    pub fn run<'a, T, E, U, F>(&self, f: F) -> impl Future<Item = T, Error = E> + Send + 'a
    where
        F: FnOnce(Connection) -> U + Send + 'a,
        U: IntoFuture<Item = (Connection, T), Error = E> + 'a,
        U::Future: Send,
        E: From<RedisError> + Send + 'a,
        T: Send + 'a,
    {
        let f = move |conn: Option<Connection>| {
            let conn = conn.unwrap();
            f(conn)
                .into_future()
                .map(|(conn, item)| (item, Some(conn)))
                .map_err(|err| (err, None))
        };
        self.pool.run(f)
    }

    /// Get a new dedicated connection that will not be managed by the pool.
    /// An application may want a persistent connection
    /// that will not be closed or repurposed by the pool.
    ///
    /// This method allows reusing the manager's configuration but otherwise
    /// bypassing the pool
    pub fn dedicated_connection(
        &self,
    ) -> impl Future<Item = Connection, Error = RedisError> + Send {
        self.pool.dedicated_connection()
            .map(|opt_con|
                opt_con.expect("Couldn't get a dedicated Redis connection!"))
    }
    /// Returns information about the current state of the pool.
    pub fn state(&self) -> bb8::State {
        self.pool.state()
    }
}

/// A `bb8::ManageConnection` for `redis::async::Connection`s.
#[derive(Clone, Debug)]
pub struct RedisConnectionManager {
    client: Client,
}

impl RedisConnectionManager {
    /// Create a new `RedisConnectionManager`.
    pub fn new(client: Client) -> Result<RedisConnectionManager> {
        Ok(RedisConnectionManager { client })
    }
}

impl bb8::ManageConnection for RedisConnectionManager {
    type Connection = Option<Connection>;
    type Error = RedisError;

    fn connect(
        &self,
    ) -> Box<Future<Item = Self::Connection, Error = Self::Error> + Send + 'static> {
        Box::new(self.client.get_async_connection().map(|conn| Some(conn)))
    }

    fn is_valid(
        &self,
        conn: Self::Connection,
    ) -> Box<Future<Item = Self::Connection, Error = (Self::Error, Self::Connection)> + Send> {
        // The connection should only be None after a failure.
        Box::new(
            redis::cmd("PING")
                .query_async(conn.unwrap())
                .map_err(|err| (err, None))
                .map(|(conn, ())| Some(conn)),
        )
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.is_none()
    }

    fn timed_out(&self) -> Self::Error {
        io::Error::new(io::ErrorKind::TimedOut, "RedisConnectionManager timed out").into()
    }
}
