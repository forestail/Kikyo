# Kikyo (桔梗)

Windows用キーボード配列エミュレータ。Tauri v2 + Rustで実装されています。

## 機能
- **配列エミュレーション**: やまぶき互換配列定義ファイル (`.yab`) を読み込み、キー入力を変換します。
- **GUI設定**: タスクトレイ・設定ウィンドウから配列ファイルの選択やON/OFFが可能。
- **安全性**: 自己注入ガードと緊急停止機能を搭載。

## ⚠️ 緊急停止方法
万が一、設定ミスやバグで入力不能になった場合、以下の操作で強制終了できます：

**`Ctrl` + `Alt` + `Esc`**

アプリが即座に終了し、入力フックが解除されます。

## 実行方法

### 必要要件
- Rust (Cargo)
- Node.js (npm)
- Windows 10/11

### 開発モードで実行
```bash
cd crates/kikyo-ui-tauri
npm install
npm run tauri dev
```

### ビルド (リリース)
```bash
npm run tauri build
```
`src-tauri/target/release/kikyo-ui-tauri.exe` が生成されます。

## 使い方
1. アプリを起動するとタスクトレイにアイコンが表示されます。
2. トレイアイコンを右クリック -> "Open Settings" または左クリックで設定画面を開きます。
3. "Path to .yab file" に `.yab` ファイルのパスを入力します。
   - 例: `C:\Users\User\Documents\新下駄.yab`
   - **注意**: ファイルは UTF-16 (BOM付き) である必要があります。
4. "Load Layout" をクリックします。
   - ステータス表示が "Loaded X sections" になれば成功です。
5. "Enable Kikyo" にチェックが入っていることを確認します。
6. メモ帳などで動作を確認してください。

## 構成
- `crates/kikyo-core`: 配列処理、フックのコアロジック (Rust Library)
- `crates/kikyo-ui-tauri`: GUIアプリ (Tauri v2)

## MVP制限事項
- 現在は「ローマ字シフト無し」のベース面のみ変換します（単打のみ）。同時打鍵（シフト面）は未実装です。
- IME連動（ModeC）は未実装です。
