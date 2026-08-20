// Windows でリリースビルド時にコンソールウィンドウを非表示にする
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ts_navigator_lib::run();
}
