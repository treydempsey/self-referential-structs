use std::boxed::Box;
use std::cell::UnsafeCell;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Future, Stream, TryStreamExt};
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

struct WrappedFuture<'f> {
    pool: UnsafeCell<SqlitePool>,
    sql: UnsafeCell<Pin<String>>,
    name: UnsafeCell<Pin<String>>,
    fetch: Option<BoxFuture<'f, Result<SqliteRow, sqlx::Error>>>,
    _pinned: PhantomPinned,
}

impl WrappedFuture<'_>
{
    fn new<'f>(pool: &'f SqlitePool, name: &str) -> Pin<Box<WrappedFuture<'f>>>
    {
        println!("WrappedFuture::new()");
        let hold_up = WrappedFuture {
            pool: UnsafeCell::new(pool.clone()),
            sql: UnsafeCell::new(Pin::new(String::from("SELECT id, name, email FROM users WHERE name = ? ORDER BY id DESC LIMIT 1"))),
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
        let query: BoxFuture<'f, Result<SqliteRow, sqlx::Error>> = Box::pin(sqlx::query(sql).bind(name).fetch_one(pool));

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
            Poll::Ready(Err(sqlx::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "fetch future is None"))))
        }
    }
}

struct WrappedStream<'s> {
    pool: UnsafeCell<SqlitePool>,
    sql: UnsafeCell<Pin<String>>,
    name: UnsafeCell<Pin<String>>,
    stream: Option<BoxStream<'s, Result<SqliteRow, sqlx::Error>>>,
    _pinned: PhantomPinned,
}

impl WrappedStream<'_>
{
    fn new<'f>(pool: &'f SqlitePool, _name: &str) -> Pin<Box<WrappedStream<'f>>>
    {
        println!("WrappedStream::new()");
        let wrapped = WrappedStream {
            pool: UnsafeCell::new(pool.clone()),
            sql: UnsafeCell::new(Pin::new(String::from("SELECT id, name, email FROM users ORDER BY name ASC"))),
            name: UnsafeCell::new(Pin::new(String::from(_name))),
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
        let _name = unsafe { &**wrapped.name.get() };
        let stream: BoxStream<'f, Result<SqliteRow, sqlx::Error>> =
            Box::pin(sqlx::query(sql)
                //.bind(name.as_str())
                .fetch(pool));

        // Safety: We haven't returned our new instance of HoldUp yet,
        // therefore we have no references. Exclusive access is safe.
        unsafe {
            let mut_ref = Pin::get_unchecked_mut(wrapped.as_mut());
            mut_ref.stream = Some(stream);
        }

        wrapped
    }
}

impl<'f> Stream for WrappedStream<'f> {
    type Item = Result<SqliteRow, sqlx::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safety: XXX What makes this use safe? XXX
        let this = unsafe { self.get_unchecked_mut() };
        if let Some(stream) = &mut this.stream {
            stream.as_mut().poll_next(cx)
        } else {
            Poll::Ready(Some(Err(sqlx::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "WrappedStream: expected stream to be Some")))))
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), sqlx::Error> {
    let pool = SqlitePool::connect("sqlite://dev.db").await.expect("connected to dev.db");

    let query = WrappedFuture::new(&pool, "Dane Petty");
    let r = query.await;
    match r {
        Ok(row) => println!("row: {}, {}, {}", sqlx::Row::get::<u32, _>(&row, 0), sqlx::Row::get::<&str, _>(&row, 1), sqlx::Row::get::<&str, _>(&row, 2)),
        Err(e) => println!("error: {:?}", &e),
    }

    let mut stream = WrappedStream::new(&pool, "_");
    while let Some(row) = stream.try_next().await? {
        println!("row: {}, {}, {}", sqlx::Row::get::<u32, _>(&row, 0), sqlx::Row::get::<&str, _>(&row, 1), sqlx::Row::get::<&str, _>(&row, 2));
    }
    
    Ok(())
}
