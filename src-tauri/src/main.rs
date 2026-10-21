#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod cli;
mod gpu;
mod llm;
mod logger;
mod models;
mod pipeline;
mod settings;
mod stt;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let test_mode = args.first().is_some_and(|a| a == "--test" || a == "--bench");
    logger::init(test_mode);
    log::info!("main reached {} ms after process creation", logger::since_process_start_ms());
    let llama_dir = std::env::current_exe()?.parent().unwrap().join("llama");
    if test_mode {
        return cli::run(&args, &llama_dir);
    }
    app::run(llama_dir)
}
