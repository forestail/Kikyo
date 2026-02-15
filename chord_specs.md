# 打鍵判定 仕様

## 1. 基本モデル

- 入力は `KeyEvent { key, edge(Down/Up), injected, t }`
- 内部状態:
  - `pressed`: 現在押下中キー
  - `pending`: 未確定キー列（`t_down`, `t_up`, `used`）
  - `used_modifiers`: 和音で消費した修飾キー
  - `prefix_pending`: `PrefixShift` 用のワンショット修飾キー
- 判定出力（`Decision`）:
  - `Passthrough`
  - `KeyTap`
  - `Chord(Vec<ScKey>)`
  - `LatchOn` / `LatchOff`（型はあるが現状ほぼ未使用）

## 2. 対象キー判定

- `profile.target_keys` が `Some` の場合、対象外キーは即 `Passthrough`
- `target_keys` はレイアウト読込時に自動再構築
  - 定義済みセル（base/sub plane）の RC から逆引き
  - トリガータグ `<...>` に含まれるキー
  - 親指キー（左/右/拡張1/拡張2）

## 3. 同時打鍵判定の重なり率

2キー判定の基本式:

`ratio = overlap_duration / second_key_duration`

- `overlap_duration`: 2キーが同時押下だった時間
- `second_key_duration`: 後から押されたキーの押下時間
- 閾値: `profile.char_key_overlap_ratio`（既定 0.35）

注意:

- `profile.thumb_shift_overlap_ratio` は現行実装では判定に使われません
  - 親指和音にも実際には `char_key_overlap_ratio` が使われます

## 4. 2キー同時

- `pending` を `t_down` 順に走査
- `ratio >= char_key_overlap_ratio` で和音成立
- 成立時:
  - `Decision::Chord([k1, k2])`
  - 修飾キーは `used_modifiers` に記録
  - `continuous=true` かつ押下中の修飾キーは pending に残す（`used=true`）
- 不成立時:
  - 古い側キーを `KeyTap` として先に確定（順序維持）

## 5. 3キー同時

- 有効条件: `profile.max_chord_size >= 3`
- 成立条件:
  - 3キー中少なくとも1キーが `Up` 済み
  - 3組すべてのペア比率が閾値以上
  - `require_modifier_for_char_chord=true` 時は修飾キーを少なくとも1つ含む
- 成立時:
  - `Decision::Chord([k1, k2, k3])`
  - 連続修飾キーは 2キー時と同様に保持

## 6. 連続シフト

### 6.1 親指連続

- 各親指設定 `thumb_left/right/extended_thumb1/extended_thumb2.continuous` を参照
- `continuous=true` かつ押しっぱなしなら次の和音へ持ち越し

### 6.2 文字キー連続

- 文字修飾キー（`trigger_keys`）を連続修飾として扱う
- ロールオーバー時に旧キーの誤出力を防ぐため以下を実装
  - 同一キー再押下時の旧 pending グループ先行 flush
  - 短すぎる重なり（`ROLLOVER_CHAIN_GUARD_OVERLAP_MS = 12ms`）の誤和音ガード
  - 未定義和音時の後段キー優先フォールバック

## 7. 単打鍵挙動

親指キーの `single_press`:

- `None`: 単打を出力しない
- `Enable`: 親指キーそのものを `KeyTap`
- `PrefixShift`: 次のキー Down を即 `Chord([thumb, next])` として扱う
- `SpaceKey`: Space（scancode `0x39`）を `KeyTap`

補足:

- その親指が和音で使用済みなら単打出力は抑制される
- 文字修飾キーも使用済みなら単打抑制される

## 8. リピート

### 8.1 発火条件

- 押下中キーに対する再度の Down をリピートイベントとして扱う

### 8.2 許可判定

- `char_key_repeat_assigned`
- `char_key_repeat_unassigned`
- トークン種別（文字割当かどうか）で分岐

### 8.3 親指リピート

- `thumb_side.repeat = true` かつ以下の場合のみ有効
  - `single_press = Enable`: 親指キーを繰り返し出力
  - `single_press = SpaceKey`: Space を繰り返し出力
- `None` / `PrefixShift` はリピート抑止

## 9. IME とセクション選択

- 実行時に `ImeMode` で日本語入力状態を判定
  - `Auto`, `Tsf`, `Imm`, `Ignore`, `ForceAlpha`
- 日本語モード時はローマ字系セクション、英数モード時は英数系セクションを選択
- 英数モードでは内部的に以下を強制
  - `require_modifier_for_char_chord = true`
  - `max_chord_size` は最大2に制限

## 10. 特記事項（実装依存）

- Space（非修飾）は Down 時に即 `Passthrough`、同時に pending を先行確定
- Enter はロールオーバー時の取りこぼし回避のため遅延パススルー処理あり
- `[機能キー]` セクションで入力キーの前段リマップを実施
  - 拡張仮想キー `Extended1..4` を内部修飾として利用可能
  - 一部ターゲットは疑似キー出力（Caps/Kana ロック）に変換
- `DirectString` は hook 側で IME OFF/ON を伴う安全出力

## 11. Profile 既定値（主要）

- `chord_window_ms = 200`（現実装では時間切れ判定に未使用）
- `max_chord_size = 2`（レイアウト解析で2/3に更新）
- `char_key_overlap_ratio = 0.35`
- `char_key_continuous = false`
- `char_key_repeat_assigned = false`
- `char_key_repeat_unassigned = true`
- `ime_mode = Auto`
- `suspend_key = None`
- `thumb_left = Muhenkan`
- `thumb_right = Henkan`
- `extended_thumb1 = Extended1`
- `extended_thumb2 = Extended2`



