
# Self Referential Futures

While writting an [actix-web](https://actix.rs/) application I decided I wanted to use
sqlx's streaming [fetch](https://docs.rs/sqlx-core/0.5.11/sqlx_core/query/struct.Query.html#method.fetch)
method to stream database results back to the client, significantly reducing memory usage
for larger datasets. This code shows a possible solution using self-referential structs.

The primary issue is one of lifetimes and ownership \(is't it always?\). A sqlx query references three things:

  * a SQL query string
  * some number of arguments bound to the query
  * a connection Pool that implements the Executor

Most importantly: the query does not own these things. It only retains references, borrows them with
associated lifetimes ensuring the future lives enough.

With typical usage the query future is awaited before the query string, arguments, and pool are
dropped. However when using actix-web the future is returned at the end of the handler and outlives
the query string, arguments, and pool.

```rust
/// This example shows a naive solution that DOES NOT WORK due to the borrow checker

#[derive(Deserialize)]
pub struct ListUsersParams {
    like: string,
}

#[get(/users)]
pub fn list_users(
    pool: web::Data<SqlitePool>,
    params: web::Query<ListUsersParams>,
) -> Result<HttpResponse> {
    let sql = String::from("SELECT id, name, email FROM users WHERE name LIKE ?");
    let stream = sqlx::query(&sql)
        .bind(&params.like)
        .fetch(&pool.clone());

    // Does not work!
    HttpResponse::Ok().streaming(convert_to_bytes_stream(stream))

    // sql, params, and pool's copy are dropped here
}

pub fn convert_to_bytes_stream(
    stream: Stream<Item = Result<SqliteRow, sqlx::Error>>,
) -> S: Stream<Item = Result<Bytes, Box<dyn Error>>> {
    // Implementation not shown
}
```

Unfortunately the stream passed to `HttpResponse::Ok().streaming()` outlives the other borrows.

## A solution

The solution I came up with was to have the stream own the query string, arguments and pool. This
codebase represents the first step towards making that happen. I've implemented a WrappedFuture and
a WrappedStream. Each are structs, owning their arguments and a future and implementing Future and stream
respectively.

Normally safe rust disallows self-references. If a struct were moved, then those references would continue
to point to the old memory locations. In order to ensure the references remain valid for the life of the
struct I'm using std::pin::Pin to disallow moves through the type system. If the struct is never moved then
it's references will remain valid. Additionally I'm using Box to allocate memory on the heap. This allows me to
return the future further up the stack from where it was created.

In order to reference other elements of Self I use [UnsafeCell](https://doc.rust-lang.org/stable/std/cell/struct.UnsafeCell.html).
It's typically used as the building block of interior mutability, but works for our case as well. The query
future depends on references to the query string, arguments, and pool. We can't construct our struct all at once.
I use an `Option<BoxFuture>` to allow me to construct our struct with everything except for the future. Then using
`Pin::get_unchecked_mut()` I can modify and update Self.

## Further work

I have yet to test this with actix-web, but I believe it should work fine. I would like to come up with a way
to abstract the owned data, query string and arguments. Perhaps turn this in to a trait and store the data in an
associated type that implemeters must define. I would also delegate query construction and argument binding to methods
that trait implementers must define.

## Questions

I'm not certain that I need `UnsafeCell`. I'd like to explore if `Cell` or `RefCell` might work instead.

Could this be solved with lifetimes alone? I don't think so. One interesting point is that `HttpResponseBuilder::streaming()` is
bound by the `'static` [lifetime](https://docs.rs/actix-web/4.0.0-rc.3/actix_web/struct.HttpResponseBuilder.html#method.streaming).
I guess they couldn't figure out how to bound it to the lifetime of the request and just stuck `'static` on it?

## Motivations for this Project

I got stuck trying to implement streaming. I tried multiple different things: implementing Stream myself but
running afoul of the borrow checker, sticking the data in Arc types, using [owning_ref](https://kimundi.github.io/owning-ref-rs/owning_ref/index.html),
searching google and finding [someone else](https://github.com/rich-murphey/sqlx-actix-streaming) implementing a
self-referential solution using [ouroboros](https://docs.rs/ouroboros/0.14.2/ouroboros/index.html).
Ultimately the ouroboros solution worked and I believe this solution with `UnsafeCell` + `Pin` will work.
I wanted to explore the Pin solution because I read it is considered the supported solution to this problem
by the core Rust developers.

I'm posting this in the hope that it might be useful for someone else.

## About Me

I'm a life long programmer who started learning Rust in mid 2020. I worked with many languages including C and C++ but found
learning Rust challenging. Specifically, I found I had to unlearn things I thought applied to Rust. Digging in writing some code between
breaks in reading the Rust book seriously helped me improve. I'm enjoying my journey with Rust. It gives me the confidence
to write things that I struggled to get right in C (threads, async networking). I'm still hoping to learn simple, usable patterns
for complex graph structures and other interconnected data structures in Rust.
