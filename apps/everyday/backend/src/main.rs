use std::io::Read;

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("--verify-inventory-act") {
        let Some(path) = arguments.get(2) else {
            eprintln!("Использование: meshkeeper-node --verify-inventory-act <акт.json>");
            std::process::exit(64);
        };
        if arguments.len() != 3 {
            eprintln!("Укажите ровно один JSON-файл акта");
            std::process::exit(64);
        }
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("Не удалось открыть акт: {error}");
                std::process::exit(66);
            }
        };
        let mut bytes = Vec::new();
        if file
            .take(12 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            eprintln!("Не удалось прочитать акт");
            std::process::exit(66);
        }
        if bytes.len() > 12 * 1024 * 1024 {
            eprintln!("Файл акта превышает 12 МиБ");
            std::process::exit(65);
        }
        let document = match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(document) => document,
            Err(_) => {
                eprintln!("Файл не является корректным JSON");
                std::process::exit(65);
            }
        };
        let report = meshkeeper_node::inventory_act::standalone_report(&document);
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("verification report")
        );
        std::process::exit(if report["cryptographicValid"] == true {
            0
        } else {
            2
        });
    }
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
