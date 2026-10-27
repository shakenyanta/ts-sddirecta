// Windows でリリースビルド時にコンソールウィンドウを非表示にする
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if !ts_navigator_lib::run_if_helper() && !ts_navigator_lib::run_if_worker() {
        ts_navigator_lib::run();
    }
}
