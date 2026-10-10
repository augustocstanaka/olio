//! The `olio` binary; everything else is in the `olio_app` library.

use olio_app::{OlioApp, USAGE, cli, parse_args};
use olio_library::Store;

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("import") {
        args.next();
        std::process::exit(cli::import(args));
    }
    let options = match parse_args(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}");
            return Ok(());
        }
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let store = match Store::open(&options.library) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("olio: cannot open the library: {err}");
            std::process::exit(1);
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Olio")
        .with_app_id("olio")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([720.0, 460.0]);
    if options.screenshot.is_some() {
        // A covered window is never painted, so it could never be captured.
        viewport = viewport
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_active(true);
    }
    let native = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "olio",
        native,
        Box::new(move |cc| Ok(Box::new(OlioApp::new(cc, store, options)))),
    )
}
