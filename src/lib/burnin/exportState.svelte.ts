// 焼き込みエクスポート実行中フラグ (グローバル)。
//
// なぜ必要か:
//   `@xpadev-net/niconicomments` は config / options / imageCache / nicoScripts を
//   **モジュールスコープの共有状態** に持つ (0.2.78 で確認。コンストラクタは毎回
//   これらを上書きし、複数インスタンス検出時には警告も出す)。
//   ライブラリ詳細ページではプレイヤーの CommentLayer が自分の NiconiComments を
//   走らせており、焼き込みエクスポートも別インスタンスを生成する。エクスポート中に
//   プレイヤー側がリサイズ等で再生成 (`new NiconiComments`) すると、共有状態が
//   プレイヤーの設定 (mode/scale/config) で上書きされ、以降の焼き込みフレームが
//   エクスポート設定ではなくプレイヤー設定で描かれてしまう (逆も起きる)。
//
//   そこでエクスポート中はこのフラグを立て、CommentLayer 側はライブ描画と
//   インスタンス生成を一時停止する。これでエクスポートのインスタンスだけが
//   モジュール状態を持ち、互いに汚染しない。
//
// プレーン TS (`browser.ts`) と Svelte ルーン (`CommentLayer.svelte`) の両方から
// 使うため、`.svelte.ts` のルーンストアにする (miniPlayerStore と同じ作法)。
//
// 単純な真偽値ではなく **実行中エクスポート数** を数える。別ウィンドウ/連打で
// 焼き込みが 2 つ重なった場合 (バックエンドは burnin_start で一意な session/出力名を
// 採るため同時実行を許容する)、先に終わった方が end() を呼んでも、残りの
// エクスポートが続く限り抑止を解除しないようにするため。

let activeCount = $state(0);

export const burnInExport = {
  /** いずれかのエクスポートが実行中か (CommentLayer がリアクティブに購読する)。 */
  get active(): boolean {
    return activeCount > 0;
  },
  /** エクスポート開始時に呼ぶ (= ライブ描画を一時停止させる)。 */
  begin(): void {
    activeCount += 1;
  },
  /** エクスポート終了時に呼ぶ (成功・失敗・キャンセルいずれも)。 */
  end(): void {
    activeCount = Math.max(0, activeCount - 1);
  },
};
