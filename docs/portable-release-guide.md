# ポータブル版のリリース手順

**v0.1.2を作って更新を試す場合は、次の1〜5だけを順番に実行してください。**
以降のリリースでは、`0.1.2` を前回より大きい番号に読み替えます。

## 1. 更新元の0.1.1を残す

プロジェクトのフォルダーでPowerShellを開き、次の1行を実行します。

```powershell
npm.cmd run update:test:prepare -- -Version 0.1.1
```

これで **`dist-update\update-test\v0.1.1\ts-sddirecta.exe`** に旧版のコピーができます。
コピー元は `dist-update\v0.1.1\ts-sddirecta.exe`、なければ `src-tauri\target\release\ts-sddirecta.exe` です。
コマンドがexeのバージョンを確認し、`0.1.1` でなければ停止します。既存のテスト用フォルダーも上書きしません。

CSV・config・input・outputの保持も試す場合は、それらのテスト用コピーをこのフォルダーに置きます。

**このexeは手順5で使います。新しいexeで上書きしないでください。**
ビルドできるWindows環境と、前回と同じ署名鍵（暗号化している場合はパスワード）が必要です。

## 2. 0.1.2の配布ファイルを作る

`package.json` の `version` だけを変更して保存します。

```json
"version": "0.1.2"
```

PowerShellで、次の2行を実行します。`-Notes` は今回の変更内容に書き換えます。

```powershell
Set-Location 'C:\Users\SYSTEM-PC7\source\repos\electron3\ts-sddirecta'
npm.cmd run release:prepare -- -Notes "アップデート動作確認用のリリースです。"
```

鍵のパスワードを求められたら入力し、**「配布ファイルの作成が完了しました」** と表示されるまで待ちます。
バージョンの同期・ビルド・署名は、このコマンドでまとめて行います。
`build:exe` や `version:sync` を別に実行する必要はありません。

生成先は `dist-update\v0.1.2\` です。公開するのは次の3ファイルです。

```text
ts-sddirecta.exe
latest.json
latest.json.sig
```

3ファイルは同じ実行で作成したものを使い、生成後の内容・名前は変更しません。
変更内容は `CHANGELOG.md` の「未リリース」欄にも日本語で記録します。記録のためのコミットは不要です。

## 3. 新しいexeの起動を確認する

`dist-update\v0.1.2\ts-sddirecta.exe` を起動し、「バージョン情報」が `0.1.2` であることと、変更した機能が動くことを確認して終了します。

## 4. GitHubに3ファイルを公開する

次の1行を実行します。

```powershell
npm.cmd run release:publish
```

`package.json` のバージョンを使い、配布先 `shakenyanta/ts-sddirecta-releases` へのタグ・タイトル設定、3ファイルの添付、最新版としての公開まで自動で行います。
説明には手順2の `-Notes` の内容を使います。**「公開が完了しました」** と表示されたら手順5へ進んでください。
**実行すると、ほかの利用者の「アップデート確認」でも取得できるようになります。**

GitHub CLIのインストールとログインは初回のみ必要です（下の補足を参照）。

## 5. 0.1.1からアップデートする

SD処理をしていない状態で、**`dist-update\update-test\v0.1.1\ts-sddirecta.exe`** を起動します。

1. 「アップデート確認」を押し、`0.1.2` への更新案内で **YES** を選びます。
2. 自動で再起動したら、「バージョン情報」が **`0.1.2`** になっていることを確認します。
3. 同じフォルダーのCSV・config・input・outputなどが保持され、アプリが正常に動くことを確認します。
4. もう一度「アップデート確認」を押し、**「最新バージョンです。」** と表示されれば確認完了です。

更新後、exeの隣の `.ts-sddirecta-update-*` フォルダーに旧exe（`previous.exe`）が残ります。動作確認が終わるまで保管してください。

## 必要なときだけ読む補足

<details>
<summary>GitHub公開の初回設定・公開前の確認・失敗時の対処</summary>

GitHub CLI（`gh`）がない場合は、次を実行してからターミナルを開き直します。

```powershell
winget install --id GitHub.cli --exact
```

配布リポジトリに書き込みできるGitHubアカウントで、初回のみログインします。

```powershell
gh auth login --hostname github.com
```

公開せずに、ローカルの配布ファイルと公開先だけ確認する場合：

```powershell
npm.cmd run release:publish -- -DryRun
```

公開コマンドは、exeと更新情報のバージョン・サイズ・SHA-256を確認し、3ファイルが存在する場合だけ処理を進めます。
下書きに3ファイルを添付し、アップロード状態とサイズが揃っていることを確認してから公開します。
署名の暗号学的な検証は更新を受け取るアプリ側で行うため、手順2で署名まで完了した3ファイルを使ってください。
既存の同じタグのリリースは上書きしません。

失敗した場合は表示されたエラーを確認してください。認証エラーはログインをやり直します。
途中で下書きが残った場合は、[Releases](https://github.com/shakenyanta/ts-sddirecta-releases/releases) で添付内容を確認します。
同じコマンドによる下書きの再開は行いません。下書きを削除してやり直すか、3ファイルを確認してGitHub画面から公開してください。
公開後の確認だけに失敗した場合も、再実行前にGitHubの公開状態を確認してください。

[GitHub CLI公式: release create](https://cli.github.com/manual/gh_release_create) / [release edit](https://cli.github.com/manual/gh_release_edit)

</details>

<details>
<summary>更新元のexeが別の場所にある場合・次回以降の更新テスト</summary>

コピー元を指定する場合：

```powershell
npm.cmd run update:test:prepare -- -Version 0.1.1 -SourceExe 'D:\アプリ\ts-sddirecta.exe'
```

次回、`0.1.2` からの更新を試す場合は、更新元の番号を指定します。

```powershell
npm.cmd run update:test:prepare -- -Version 0.1.2
```

既にテスト用フォルダーがある場合は、その中のexeを使ってください。
更新済みなら「バージョン情報」で確認し、再テストに必要なファイルを退避してからテスト用フォルダーを削除し、コマンドを再実行します。
このコマンドはコピーだけを行うため、更新機能を備えた旧版の配布用exeが必要です。

</details>

<details>
<summary>署名鍵の保存先・標準以外の場所を使う場合</summary>

標準の保存先は次の2つです。前回と同じ鍵を使い続け、リリースのたびに作り直さないでください。

- `$env:USERPROFILE\.tauri\ts-sddirecta.key`
- `$env:USERPROFILE\.tauri\ts-sddirecta.key.pub`

秘密鍵とパスワードはバックアップし、リポジトリやリリースに公開しないでください。
アプリには公開鍵 `src-tauri/update-public-key.txt` だけを組み込みます。

標準以外の保存先を使う場合は、手順2の作成コマンドを次に置き換えます。
公開鍵も、秘密鍵のパスに `.pub` を付けた場所に必要です。

```powershell
npm.cmd run release:prepare -- -PrivateKeyPath 'D:\署名鍵\ts-sddirecta.key' -Notes "今回の変更内容"
```

</details>

<details>
<summary>エラーや、更新が案内されない場合</summary>

| 状況 | 確認・対処 |
| --- | --- |
| バージョンの同期でエラー | `package.json` の構文と `version` を確認。`0.1.2` のように先頭の `v` は付けない。 |
| 秘密鍵がない／公開鍵が一致しない | 前回の鍵の保存先を確認し、必要なら `-PrivateKeyPath` で指定する。新しい鍵へ作り替えない。 |
| ビルド・署名が失敗した | エラーを解消し、手順2のコマンドを再実行する。以前の生成物を公開しない。 |
| GitHubの認証でエラー | `gh auth login --hostname github.com` で、配布リポジトリに書き込めるアカウントへログインする。 |
| 同じタグのリリースが既にある | GitHubの公開状態を確認。公開済みなら手順5へ進む。失敗時の下書きなら上の補足を参照。 |
| 「更新情報はまだ公開されていません。」と出る | 公開済みの最新版のAssetsに `latest.json` があるか確認する。 |
| 更新前から「最新バージョンです。」と出る | 起動中のexeが `0.1.1`、GitHubのLatestが `0.1.2` か確認する。同じか古い番号には更新されない。 |
| 署名・ハッシュの検証に失敗する | 同じ実行で生成した3ファイルか、生成後に内容を変更していないか確認する。 |
| exeを差し替えられない | 保存先への書込権限、同じexeの別起動、ファイルの使用中状態を確認する。 |

公開後に修正する場合は、公開済みの番号を使い回さず、さらに大きいバージョンで作成します。
差し替え・新exeのプロセス起動に失敗した場合は旧exeへの復元を試みますが、起動後の動作異常までは自動判定しません。

</details>

<details>
<summary>自動処理の内容・利用者への案内</summary>

- バージョンは `package.json` から `src-tauri/Cargo.toml`・`src-tauri/tauri.conf.json`・`package-lock.json` のアプリ自身のバージョンへ自動反映します。
- `latest.json` にバージョン・対象CPU・exeのURL・サイズ・SHA-256・変更内容を記録し、そのJSON全体に署名します。
- 更新は配布用ビルドで行います。開発起動では更新確認のみ可能です。現在の配布スクリプトは1リリースにつき1つのWindows CPU構成を対象にします。
- 更新を見送る操作も確認する場合は、手順5の最初に **No** を選び、旧版のままであることを確認してから、再度更新します。

利用者への案内例：

```text
ts-sddirecta v0.1.2 を公開しました。

既にお使いの方：アプリの「アップデート確認」→ YESで更新できます。
初めてお使いの方：下記ページのAssetsから ts-sddirecta.exe をダウンロードして起動してください。
https://github.com/shakenyanta/ts-sddirecta-releases/releases/latest
```

</details>
