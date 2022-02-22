use std::boxed::Box;
use std::cell::UnsafeCell;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use futures::Future;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

pub struct WrappedFuture<'f> {
    pool: UnsafeCell<SqlitePool>,
    sql: UnsafeCell<Pin<String>>,
    name: UnsafeCell<Pin<String>>,
    fetch: Option<BoxFuture<'f, Result<SqliteRow, sqlx::Error>>>,
    _pinned: PhantomPinned,
}

impl WrappedFuture<'_> {
    pub fn new<'f>(pool: &'f SqlitePool, name: &str) -> Pin<Box<WrappedFuture<'f>>> {
        println!("WrappedFuture::new()");
        let hold_up = WrappedFuture {
            pool: UnsafeCell::new(pool.clone()),
            sql: UnsafeCell::new(Pin::new(String::from(
                "SELECT id, name, email FROM users WHERE name = ? ORDER BY id DESC LIMIT 1",
            ))),
            name: UnsafeCell::new(Pin::new(String::from(name))),
            fetch: None,
            _pinned: PhantomPinned,
        };
        let mut boxed = Box::pin(hold_up);

        // Safety: Shared access to this.pool is safe because we are using a
        // cloned copy of the argument pool, and SqlitePool is a newtype around
        // and Arc containing the connection. It will continue to exist so long
        // as our ownership in boxed.pool exists.
        let pool = unsafe { &*boxed.pool.get() };
        // Safety: same as pool
        let sql = unsafe { &**boxed.sql.get() };
        // Safety: same as pool
        let name = unsafe { &**boxed.name.get() };
        let query: BoxFuture<'f, Result<SqliteRow, sqlx::Error>> =
            Box::pin(sqlx::query(sql).bind(name).fetch_one(pool));

        // Safety: We haven't returned our new instance of HoldUp yet,
        // therefore we have no references. Exclusive access is safe.
        unsafe {
            let mut_ref = Pin::get_unchecked_mut(boxed.as_mut());
            mut_ref.fetch = Some(query);
        }

        boxed
    }
}

impl<'f> Future for WrappedFuture<'f> {
    type Output = Result<SqliteRow, sqlx::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: XXX What makes this use safe? XXX
        let this = unsafe { self.get_unchecked_mut() };
        if let Some(query) = &mut this.fetch {
            query.as_mut().poll(cx)
        } else {
            Poll::Ready(Err(sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "fetch future is None",
            ))))
        }
    }
}
