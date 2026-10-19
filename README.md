# ts-sddirecta

## 配布用exeの作成

```powershell
npm.cmd run build:exe
```

インストーラーを作成せず、`src-tauri/target/release/ts-sddirecta.exe` を生成します。
生成したexeを使用したいフォルダーにコピーして起動してください。
`panel_sd` には、起動したexeが存在するフォルダーを表示します。

実行先のPCには Microsoft Edge WebView2 Runtime が必要です。
画面用のHTML・CSS・JavaScriptはリリースビルドのexeに組み込まれるため、
`dist` や `node_modules` を配布する必要はありません。

## 開発用の起動

```powershell
npm.cmd run tauri -- dev
```
