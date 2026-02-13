# Kikyo (桔梗)

Windows 向けのキーボード配列エミュレータです。  
`Tauri v2 + Rust` で実装されており、`.yab` / `.bnz` レイアウトを読み込んでキー入力を変換します。

## 現在の実装状況（2026-02 時点）

- 配列読み込み
  - `.yab` / `.bnz` ファイル選択と読み込み（GUI）
  - レイアウト名の取得とトレイ/ウィンドウタイトル反映
  - `UTF-8` / `BOM付き` / `Shift_JIS` のデコードに対応
- 入力エンジン
  - 親指シフト（左/右）＋拡張親指シフト（1/2）
  - 文字キー同時打鍵（Chord）判定
  - 連続シフト（ロールオーバー）と重なり率しきい値調整
  - 単独打鍵動作（無効 / 有効 / 前置シフト / Space）
  - キーリピート制御（割り当てあり/なし、親指キー側）
- レイアウト機能
  - サブプレーン `<...>` による修飾打鍵
  - `[機能キー]` セクションによるキー差し替え
  - 仮想拡張キー `拡張1..4`（`Extended1..4`）を入力元キーとして利用可能
- 動作制御
  - IMEモード切替（`Auto` / `Tsf` / `Imm` / `Ignore`）
  - Suspendキーで有効/無効トグル（`ScrollLock`, `Pause`, `Insert`, `RightShift`, `RightControl`, `RightAlt`）
  <!-- - 緊急停止 `Ctrl + Alt + Esc` -->
- デスクトップアプリ機能
  - タスクトレイ常駐（表示・再読み込み・有効切替・終了）
  - ウィンドウを閉じても終了せず、トレイへ格納
  - シングルインスタンス（多重起動時は既存ウィンドウを前面化）
  - Windows ログオン時自動起動（UIからON/OFF）
  - 設定保存（`settings.json`）

## 必要環境

- Windows 10/11
- Rust（Cargo）
- Node.js / npm

## 起動方法（開発）

```bash
cd crates/kikyo-ui-tauri
npm install
npm run tauri dev
```

## ビルド（リリース）

```bash
cd crates/kikyo-ui-tauri
npm run tauri build
```

主な生成物:

- `crates/kikyo-ui-tauri/src-tauri/target/release/kikyo-ui-tauri.exe`

## テスト

```bash
cargo test -p kikyo-core
```

## 使い方（最短）

1. 起動後、設定画面で配列ファイル（`.yab` / `.bnz`）を読み込む
2. 必要に応じて親指シフト・同時打鍵・IMEモード・Suspendキーを調整
3. 画面上部トグルまたはトレイメニューで有効化
4. 設定画面を閉じてもバックグラウンドで動作（終了はトレイメニューから）

## 設定保存

アプリは以下を `settings.json` に保存します。

- 最後に読み込んだレイアウトファイルパス
- 入力プロファイル（親指シフト・同時打鍵・IMEモード・Suspendキー等）

## ワークスペース構成

- `crates/kikyo-core`: 入力エンジン・フック・IME判定・レイアウトパーサ
- `crates/kikyo-ui-tauri`: Tauri UI（フロントエンド + バックエンド）

<!--
## 緊急停止

万が一制御不能になった場合は次で即時終了できます。

- `Ctrl + Alt + Esc`
-->

## 連絡先

- X (Twitter): [https://x.com/SurikireOfGokou](https://x.com/SurikireOfGokou)
- Email: [forestailjp@gmail.com](mailto:forestailjp@gmail.com)
