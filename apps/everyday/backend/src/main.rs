fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(error) = runtime.block_on(meshkeeper_node::run()) {
        eprintln!("Узел остановлен: {error:#}");
        std::process::exit(1);
    }
}
