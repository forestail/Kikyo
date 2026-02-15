# Kikyo

Windows 向けのキーボード入力変換アプリです。  
`Tauri v2 + Rust` で実装されており、レイアウトファイル（`.kky` / `.yab` / `.bnz`）を読み込んで入力を変換します。

## 現在の実装状態（2026-02-15 時点）

- レイアウト読込
  - `.kky` / `.yab` / `.bnz` を GUI から選択して読み込み可能
  - UTF-8 / BOM 付き / Shift_JIS の自動判定デコード対応
  - 複数レイアウトを登録・並べ替え・切替可能（設定画面 + トレイ）
- 入力エンジン
  - 親指シフト（左/右）と拡張親指（1/2）
  - 親指シフト / 文字キー連続シフト判定
  - 2キー同時打鍵 / 3キー同時打鍵判定
  - 機能キー入替（`[機能キー]` セクション）
  - IME 判定モード（`Auto` / `Tsf` / `Imm` / `Ignore`）
- アプリ運用
  - 常駐トレイ（有効/無効切替、レイアウト切替、再読込、終了）
  - ウィンドウを閉じても終了せずトレイへ格納
  - Windows 自動起動トグル
  - 単一起動（2重起動時は既存ウィンドウを前面化）

## 動作要件

- Windows 10/11
- Rust（cargo）
- Node.js / npm

## 開発実行

```bash
cd crates/kikyo-ui-tauri
npm install
npm run tauri dev
```

## ビルド

```bash
cd crates/kikyo-ui-tauri
npm run tauri build
```

## テスト

```bash
cargo test -p kikyo-core
```

## 設定ファイル

- アプリ設定は `settings.json` に保存されます（Tauri の app config dir）。
- 主な保存項目:
  - レイアウト一覧（`layout_entries`）
  - 現在のアクティブレイアウト（`active_layout_id`）
  - プロファイル（`profile`）
  - 有効/無効状態（`enabled`）
- 旧キー `last_yab_path` は起動時に `last_layout_path` へ移行されます。

## レイアウト仕様（要点）

- セクション: `[セクション名]`
- サブプレーン: `<q>`, `<q><w>` のようなタグ
- セル値:
  - `xx` / `無` / 空: 未定義
  - 通常文字列: キー列として展開
  - `'...'`: キー列展開（かな→ローマ字変換など）
  - `"..."`: 直接文字列出力
  - `機10`: F10 など
  - `V1B`: 仮想キー（16進）
- 機能キー入替:
  - `[機能キー]` セクションに `元キー,先キー` を列挙

詳細は `chord_specs.md` を参照してください。

## ワークスペース構成

- `crates/kikyo-core`: 入力エンジン、フック、IME、パーサ
- `crates/kikyo-ui-tauri`: 設定 UI / トレイ / 永続化 / Tauri 統合
- `layout`: サンプル配列ファイル

