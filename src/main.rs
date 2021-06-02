mod core;
mod err;

use std::sync::Arc;

use failure::Fallible;
use hyper::{Server, server::conn::AddrStream, service::{make_service_fn, service_fn}};

use crate::core::Handler;

#[tokio::main]
async fn main() -> Fallible<()> {
    let handler = Arc::new(Handler::new().await?);

    let bind_addr = handler.bind_addr()?;

    ////////////////////////////////////////////////////////////////////////////////////////////////////

    let make_service = make_service_fn(
        move |conn: &AddrStream| {
            let addr = conn.remote_addr();
            let handler = handler.clone();
            async move {
                let addr = addr.clone();
                Ok::<_, hyper::Error>(service_fn(move |req| {
                    let handler = handler.clone();
                    async move { handler.clone().serve(req, addr.clone()).await }
                }))
            }
        }
    );
    
    println!("Serve");
    let server = Server::bind(&bind_addr).serve(make_service);

    ////////////////////////////////////////////////////////////////////////////////////////////////////

    let graceful = server.with_graceful_shutdown(shutdown_signal());
    if let Err(e) = graceful.await {
        println!("server error: {}", e);
    }

    Ok(())
}

async fn shutdown_signal() {
    // Wait for the CTRL+C signal
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
}
