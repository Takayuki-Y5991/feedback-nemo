# Feedback Memo

日々の出来事を「日時・人・内容」の3項目で素早く残し、四半期末のフィードバック材料として人ごとに振り返るためのローカルファーストなデスクトップアプリです。

## 現在の実装

- Tauri 2によるmacOS / Linux向けデスクトップ構成
- 右下に表示するクイック入力パネル
- ヘッダーのドラッグ移動と、SQLiteによる表示位置の復元
- 2〜3行の短い記録に合わせたコンパクトな入力欄
- `人 → 日時 → 内容 → 保存` のTab移動
- macOS: `Cmd + Shift + M`、Linux: `Ctrl + Shift + M`
- システムトレイからのクイック入力・振り返り・終了
- SQLiteへの人物・メモ保存
- 入力途中の下書き保持
- 人・キーワードによる履歴絞り込み
- メンバー追加設定

SQLiteファイルはOS標準のアプリデータディレクトリ配下に `feedback-memo.sqlite3` として保存されます。

## 開発環境

- Node.js 20以降
- Rust stable
- Tauri 2のLinuxまたはmacOS向けシステム依存パッケージ

```bash
pnpm install
pnpm tauri dev
```

Web UIのみを確認する場合:

```bash
pnpm dev
```

ブラウザ確認時はSQLiteの代わりに一時的なサンプルデータを使います。

## 検証

```bash
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
```

## 次の実装候補

- 人の無効化・並べ替え
- メモ編集と削除Undo
- 四半期フィルター
- Markdown / CSVエクスポート
- 右上・右下・前回位置の比較と設定化
- AppImage / deb / dmgの配布ビルド

プロダクト計画は [PRODUCT_PLAN.md](./PRODUCT_PLAN.md) を参照してください。
