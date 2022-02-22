use std::boxed::Box;
use std::cell::UnsafeCell;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::BoxStream;
use futures::Stream;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

pub struct WrappedStream<'s> {
    pool: UnsafeCell<SqlitePool>,
    sql: UnsafeCell<Pin<String>>,
    name: UnsafeCell<Pin<String>>,
    stream: Option<BoxStream<'s, Result<SqliteRow, sqlx::Error>>>,
    _pinned: PhantomPinned,
}

impl<'s> WrappedStream<'s> {
    pub fn new(pool: &'s SqlitePool, name: &str) -> Pin<Box<WrappedStream<'s>>> {
        println!("WrappedStream::new()");
        let wrapped = WrappedStream {
            pool: UnsafeCell::new(pool.clone()),
            sql: UnsafeCell::new(Pin::new(String::from(
                "SELECT id, name, email FROM users WHERE name LIKE ? ORDER BY name ASC",
            ))),
            name: UnsafeCell::new(Pin::new(String::from(name))),
            stream: None,
            _pinned: PhantomPinned,
        };
        let mut wrapped = Box::pin(wrapped);

        // Safety: Shared access to this.pool is safe because we are using a
        // cloned copy of the argument pool, and SqlitePool is a newtype around
        // and Arc containing the connection. It will continue to exist so long
        // as our ownership in boxed.pool exists.
        let pool = unsafe { &*wrapped.pool.get() };
        // Safety: same as pool
        let sql = unsafe { &**wrapped.sql.get() };
        // Safety: same as pool
        let name = unsafe { &**wrapped.name.get() };
        let stream: BoxStream<'s, Result<SqliteRow, sqlx::Error>> =
            Box::pin(sqlx::query(sql).bind(name).fetch(pool));

        // Safety: We haven't returned our new instance of HoldUp yet,
        // therefore we have no references. Exclusive access is safe.
        unsafe {
            let mut_ref = Pin::get_unchecked_mut(wrapped.as_mut());
            mut_ref.stream = Some(stream);
        }

        wrapped
    }
}

impl<'s> Stream for WrappedStream<'s> {
    type Item = Result<SqliteRow, sqlx::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safety: XXX What makes this use safe? XXX
        let this = unsafe { self.get_unchecked_mut() };
        if let Some(stream) = &mut this.stream {
            stream.as_mut().poll_next(cx)
        } else {
            Poll::Ready(Some(Err(sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "WrappedStream: expected stream to be Some",
            )))))
        }
    }
}
