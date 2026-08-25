> This is the canonical Japanese README. For English, see [README.en.md](./README.en.md).

<div align="center">

<img src="./docs/logo.png" alt="Munder Difflin。自分の分身となるエージェントチームを動かすハーネス" width="340">

# Munder Difflin

### 普段使っているコーディングCLIを、自分の代わりに動くエージェントチームへ

**無料かつオープンソースのマルチエージェント基盤**です。
すでに契約しているサービスの時間単位の利用上限を使い、普段のターミナル型コーディングCLIを、自分が離れている間も作業を続ける分身へ変えます。
複数のエージェントは自分のマシン上で動き、Michaelがチーム全体をまとめます。

[Claude Code](https://claude.com/claude-code)、Antigravity（Gemini）、OpenAI Codex、**xAI Grok**、**Kimi Code**、**Gemini CLI**、**Qwen**、**OpenCode**、**Crush**、**pi.dev**、**GitHub Copilot CLI**、**Cursor**に対応します。
自分のAPIキーとローカルLLMも利用できます。

<p>
  <em>Electron · React · TypeScript · Pixi.js · xterm.js · node-pty</em>
</p>

<p>
  <a href="./LICENSE"><img alt="ライセンス: MIT" src="https://img.shields.io/badge/license-MIT-F4D35E.svg?style=flat-square&labelColor=6E1423"></a>
  <a href="./CHANGELOG.md"><img alt="バージョン: 0.4.5" src="https://img.shields.io/badge/version-0.4.5-F4D35E.svg?style=flat-square&labelColor=6E1423"></a>
  <img alt="状態: 動作するプロトタイプ" src="https://img.shields.io/badge/status-working%20prototype-F4F1EA.svg?style=flat-square&labelColor=6E1423">
  <img alt="対応OS: macOS | Windows | Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-F4F1EA.svg?style=flat-square&labelColor=6E1423">
  <a href="./CONTRIBUTING.md"><img alt="Pull Request歓迎" src="https://img.shields.io/badge/PRs-welcome-F4D35E.svg?style=flat-square&labelColor=6E1423"></a>
  <a href="https://discord.gg/SEDzP5ZPk5"><img alt="Discord" src="https://img.shields.io/badge/Discord-join%20the%20office-F4D35E.svg?style=flat-square&labelColor=6E1423"></a>
</p>

<br>

<img src="./docs/media/og.png" alt="メッセージを送り、作業を振り分け、記憶を共有するMunder Difflinのエージェントチーム" width="1240">

<br>

<!-- GitHub上で再生するにはraw URLが必要です。相対パスではリンクとして扱われます。 -->
<video src="https://github.com/chaitanyagiri/munder-difflin/raw/main/docs/media/hero.mp4" poster="https://github.com/chaitanyagiri/munder-difflin/raw/main/docs/media/og.png" controls muted loop playsinline width="820">
  <a href="https://github.com/chaitanyagiri/munder-difflin/raw/main/docs/media/hero.mp4">▶ Claude Codeのエージェントチームが動く様子を見る</a>
</video>

</div>

---

> [!NOTE]
> **最高のエージェントを、最低の製紙会社へ。**
> Munder Difflinは、普段使っている`claude`、`agy`、`codex`、`grok`、`kimi`、`qwen`、`opencode`、`crush`、`pi`、`copilot`を連携するチームへ変えます。
> 各エージェントには長期記憶、受信箱、2Dオフィス上の机があります。
> 分身となるMichaelが作業を振り分けますが、最終的な管理者は利用者です。

## 目次

- [どんな課題を減らすか](#どんな課題を減らすか)
- [向いている場面](#向いている場面)
- [最短で試す](#最短で試す)
- [仕組み](#仕組み)
- [主な機能](#主な機能)
- [v0.4.5の状態と制限](#v045の状態と制限)
- [アーキテクチャ](#アーキテクチャ)
- [プロジェクト構成](#プロジェクト構成)
- [デザインシステム](#デザインシステム)
- [ロードマップ](#ロードマップ)
- [安全、テレメトリー、復旧](#安全テレメトリー復旧)
- [コントリビューション](#コントリビューション)
- [ライセンス](#ライセンス)
- [謝辞](#謝辞)

## どんな課題を減らすか

複数のコーディングエージェントを同時に動かすと、作業の割り当て、会話の引き継ぎ、進捗の確認が別々の操作になります。
セッションを閉じれば学んだ内容も散らばり、並行作業では同じブランチを触って衝突することもあります。

Munder Difflinは、実際のターミナルプロセスをそのまま使いながら、連絡、記憶、作業台帳、任意のgit worktree分離を一つのデスクトップアプリへまとめます。
利用者はMichaelへ依頼し、必要なら各ターミナルへ直接入力できます。

ただし、これは完成した業務基盤ではありません。
現在の状態はv0.4.5の動作するプロトタイプです。

## 向いている場面

- すでに対応するコーディングCLIを使っており、複数の作業を並行させたい場合。
- エージェント間の連絡、記憶、タスク、ファイル、git差分を一つの画面で確認したい場合。
- APIキーやローカルLLMを自分で管理し、処理を自分のマシン上で動かしたい場合。

次の用途には慎重な評価が必要です。

- 完成済みの企業向け運用製品や、無人での破壊的操作を求める場合。
- 対応CLI、Node.js 18以降、`node-pty`をビルドできるC/C++ツールチェーンを用意できない環境。
- プロンプト、コード、ファイルパス、エージェント出力を含むテレメトリーを想定している場合。本製品はそれらを送信しません。

## 最短で試す

### 前提条件

- **macOS、Windows、Linux**のいずれか。
- **Node.js 18以降**とnpm。
- `node-pty`のネイティブアドオンをビルドできる**C/C++ツールチェーン**。
  macOSではXcode Command Line Toolsを導入します。

  ```bash
  xcode-select --install
  ```

- `PATH`上に、対応するエージェントCLIが一つ以上必要です。
  既定は[Claude Code](https://claude.com/claude-code)の`claude`です。
  ほかに`agy`、`codex`、`grok`、`kimi`、`gemini`、`qwen`、`opencode`、`crush`、`pi`、`copilot`、`cursor-agent`を利用できます。
  不足しているCLIの多くは、ハーネスがターミナルでインストーラーを実行してから新しいバイナリを起動します。
- 任意で「設定」→「AIエンジン」からAPIキーやOllama、LM Studio、vLLMのURLを設定できます。
- セッションをまたぐ意味検索用の索引は任意です。
  Markdown形式の記憶は索引がなくても動作します。

### インストールと起動

```bash
git clone https://github.com/chaitanyagiri/munder-difflin.git
cd munder-difflin
npm install        # ElectronのABIに合わせてnode-ptyを再ビルド
npm run dev        # ホットリロード付きでElectronアプリを起動
```

初回起動では設定ウィザードが開きます。
設定後、「エージェントを追加」から最初のセッションを起動すると、GODエージェントがMichaelのオフィスへ自動で配置されます。

```text
クローンして依存関係を導入
        ↓
初回ウィザードで作業フォルダーとMichaelを設定
        ↓
エージェントを追加し、Michaelへ作業を依頼
        ↓
フロア、ターミナル、タスク、git差分で確認
```

この流れは、ローカルへ導入し、初回設定を済ませ、エージェントを起動して結果を確認する4段階です。

### そのほかのコマンド

```bash
npm run build      # electron-viteによる本番ビルド
npm run preview    # 本番ビルドをプレビュー
npm run typecheck  # main/preloadとrendererを型検査
```

Electronの更新後に`node-pty`を読み込めない場合は、`npm install`を再実行してください。
`postinstall`が現在のElectron ABIに合わせて`electron-rebuild`を実行します。

## 仕組み

```text
          利用者 ── 依頼 ──►  ┌─────────────┐
                              │ GODエージェント│  統括と監督
                              │  （Michael）  │  名簿、振り分け、判断
                              │               │  共有ボード、タスク台帳
                              └──────┬──────┘
                                     │ 割り当て、転送、確認依頼
            ┌────────────────────────┼────────────────────────┐
            ▼                        ▼                        ▼
      ┌───────────┐            ┌───────────┐            ┌───────────┐
      │エージェントA│  メッセージ │エージェントB│  メッセージ │エージェントC│
      │プロバイダー │ ─────────► │プロバイダー │ ─────────► │プロバイダー │
      │   と記憶   │            │   と記憶   │            │   と記憶   │
      └───────────┘            └───────────┘            └───────────┘
            └──── 共有Hive：記憶、受信箱、共有ボード、ログ ─────┘
```

図のとおり、利用者はMichaelへ依頼し、Michaelが複数の実ターミナルプロセスへ作業を割り当てます。

1. **エージェントを起動します。**
   各エージェントは`claude`、`agy`、`codex`などの通常のターミナルプロセスです。
   固有の作業ディレクトリ、識別情報、プロバイダー別のライフサイクルを持ちます。
2. **Hiveを介して連携します。**
   Hiveはプレーンファイルを保存するローカルgitリポジトリです。
   エージェントは自分の`outbox/`へ書き、ハーネスのルーターが送信先の`inbox/`へ届けます。
   エージェント自身はgitを操作しません。
   単一のコミッターに限定することで`index.lock`の破損を避けます。
3. **GODエージェントがフロアを運営します。**
   通常の依頼は自律的に処理し、支出、破壊的操作、スコープ変更など重要な項目だけを承認キューへ送ります。
4. **状態を画面で確認できます。**
   アバター、封筒、ターミナル出力を見ながら、各セッションへ入力し、ファイルとgit履歴を確認できます。

詳しい設計は[`HIVE.md`](./HIVE.md)、ターミナルとイベントの仕様は[`SPEC.md`](./SPEC.md)、画面設計は[`DESIGN.md`](./DESIGN.md)にあります。

## 主な機能

### オフィスフロア

- Claude Code、Antigravity、OpenAI Codex、xAI Grok、Kimi Code、Gemini CLI、Qwen、OpenCode、Crush、pi.dev、GitHub Copilot CLI、Cursor、カスタムコマンドを、`node-pty`のPTYとして起動し、xterm.jsで表示します。
- Pixi.jsのオフィスでは、実際の作業状態に応じてアバターが移動し、メッセージを封筒で表現します。
- GODエージェントへ文字または音声で依頼できます。
- エージェントごとにgit worktreeを分け、並行作業の衝突を避けられます。

### 記憶と連携

- エージェント別の記憶、原子的に更新するメールボックス、共有ボード、追記専用イベントログ、単一コミッターのgitでHiveを構成します。
- Markdownの記憶を共有の検索索引へ取り込み、長期化した記憶を圧縮します。
- Enterprise Knowledge Graphへ文書やポリシーを登録し、どのエージェントからも検索できます。

### 制御と安全機能

- 支出、スコープ変更、破壊的操作は人の確認へ送ります。
  実行中の指示変更と段階的な停止もできます。
- ループ、エラーの連発、予算超過には「指示変更、制限、停止」の順でサーキットブレーカーが対応します。
- エージェント別トークン予算、トランスクリプト由来の実コスト、永続台帳、OpenTelemetry span、ツール実行履歴を表示します。

### コマンドセンター

- 依存関係付きKanban、スケジュール実行、ハートビート、チーム監視、記憶検索、活動ログ、CI監視を備えます。
- Claude Code、OpenCode、Codexの導入済みスキルを確認できます。
  227件のカタログから検索、絞り込み、導入、削除もできます。
- Monaco IDEでファイルツリー、タブ、保存、CHANGES、HISTORY、COMPARE、コミットグラフ、差分、ブランチ比較、保護付きcheckoutを扱えます。
  ファイルとgitへのアクセスはすべてmain processが仲介します。

### 外部からの依頼と成果の取り出し

- SlackまたはWebhookから依頼すると、Michaelが一時ワーカーを起動し、スレッドへ返信して終了できます。
- `munderdifflin://hire`リンクから役割を取り込めます。
  インポートはフォームを埋めるだけで、人が明示的に起動します。
  役割は[Agent Gallery](https://munderdiffl.in/hires/)でも探せます。
- プロバイダー別APIキーは書き込み専用のsecret brokerへ保存します。
  Ollama、LM Studio、vLLMのbase URLにも対応します。
  [オープンモデルのガイド](https://munderdiffl.in/blog/run-munder-difflin-on-open-models/)と[Mac Miniのガイド](https://munderdiffl.in/blog/run-munder-difflin-on-a-mac-mini/)も参照できます。
- タイトルバーから現在のマシン向け更新を取得し、インストール手順を確認できます。
  最新版なら確認後に`latest`と表示し、更新後の初回起動ではデザイン済みのリリースノートを開きます。
  バックグラウンド自動更新は「設定」にあります。
- 「前提ツール」画面でuv、git、Node、MemPalace、各エージェントCLIの状態と用途を確認し、不足分の導入をMichaelへ依頼できます。

## v0.4.5の状態と制限

v0.4.5では、信頼できるように見えていた三つの問題を修正しました。

- アプリ再起動のたびにコスト集計がリセットされる一方でセッションIDは変わらず、実際の支出より少なく表示されていました。
  現在は台帳から合算し、セッション単位の値も別に保持します。
- Apple SiliconではCoreMLが量子化された埋め込みグラフでオーバーフローし、すべてのベクトルがNaNとなって登録を拒否されていました。
  macOSでは埋め込み処理をCPUへ固定しています。
- エージェント間の連絡が安定せず、誰も起動されない受信箱へメールが残ることがありました。
  受信箱の起動監視、古い通知の除去、存在しない受信箱へのメールを差し戻して記録する処理を追加しました。

同じリリースには、平日の時刻指定トリガー、全ターミナルのクリック可能なパス、エディターの一本化、ワンクリック更新、Chromium sandbox内のrendererも含まれます。
コミュニティから23件のPull Requestが取り込まれました。

**0.3.8を使っている場合は更新してください。**
同版の利用上限ガードは保留したエージェントを解放できず、現在は機能自体が削除されています。
署名およびnotarize済みのmacOS版、Windows版、Linux版は[リリースページ](https://github.com/chaitanyagiri/munder-difflin/releases/latest)にあります。

## アーキテクチャ

二つのデータプレーンが一つのrendererへ情報を渡します。

```text
┌───────────────────────────────────────────────────────────────┐
│                    Electron Renderer（React）                 │
│   ┌──────────────────┐    ┌──────────────────────────────┐    │
│   │ オフィスフロア    │    │ ターミナルとコマンド入力      │    │
│   │ （Pixi.js）       │    │ ファイルとGit（xterm.js）    │    │
│   └─────────▲────────┘    └────────────▲─────────────────┘    │
│             │ アバター状態             │ PTY、fs、git          │
└─────────────┼──────────────────────────┼───────────────────────┘
              │ IPC（contextBridge: window.cth）
       ┌──────┴──────────┐        ┌──────┴─────────────┐
       │ イベントプレーン │        │ ターミナルプレーン   │
       │ hooksとHive     │        │ node-pty PTY        │
       │ routerとGOD     │        │ fsとgit             │
       └────────▲────────┘        └──────▲─────────────┘
                │ hook payload           │ stdinとstdout
                └─────────┬──────────────┘
                   ┌──────┴──────────────┐
                   │ claude / agy / codex│
                   └─────────────────────┘
```

図が示すのは、Electronのmain processがイベント処理とPTY処理を所有し、型付きの`window.cth`だけをrendererへ公開する境界です。

- **ターミナルプレーン**：main processの`PtyManager`がエージェントを`node-pty`で起動し、ID別IPC（`pty:data:<id>`）で出力を渡します。
  rendererは[`src/preload/index.ts`](./src/preload/index.ts)の型付きbridgeだけを使い、sandbox化されたファイルとgitの補助機能へアクセスします。
- **Hiveとイベントプレーン**：`hive.ts`がディスク上のマルチエージェント層を構成します。
  `hooks.ts`のhook serverは、Claude Codeの`cth-hook`とAntigravityの`agy-hook`からライフサイクル情報を受け取ります。
  `memory.ts`は意味検索CLIを包みます。
  ルーター、GODエージェント、アイドル時と受信時の起動処理がメッセージを届けます。

## プロジェクト構成

```text
src/
  main/                      Electron main process（Node）
    index.ts                 window、IPC handler、終了保護
    pty.ts                   node-pty管理
    hive.ts                  記憶、メールボックス、router
    hooks.ts                 hook serverとprovider shim
    memory.ts                意味検索層。利用不能時は何もしない
    config.ts                設定の永続化とホーム設定
    transcript.ts            ~/.claude/projects/のJSONLからtokenとcostを取得
    telemetry.ts             OpenTelemetry collectorとusage/cost feed
    usage.ts / pricing.ts    UsageProviderとmodel別cost
    breaker.ts / control.ts  サーキットブレーカーと人の確認、指示、停止
    reflect.ts               記憶の圧縮
    db.ts                    SQLiteによるwindow、履歴、cost台帳の永続化
    github.ts                gh CLIからGitHub IssueとCI runを取得
    shellEnv.ts              child process用PATHとshell環境
    fs.ts / git.ts           sandbox化されたfsとgitのbridge
  preload/                   contextBridgeから型付きwindow.cth APIを公開
  renderer/src/
    App.tsx                  最上位layoutと接続
    design/                  design tokenとglobal CSS
    components/              panel、agent詳細、command、承認、記憶など
    CommandCenterPanel,      MichaelのTerminal、Floor、Memory、Activity、Tasks、Triggers、Handbook
    ToolWaterfall,           agent別tool span
    TasksKanban,             依存関係付きKanban
    ThreadsPanel,            Hiveメッセージviewer
    MessageQueueComposer,    busyなagentへのmessage待機
    scene/office/            Pixi office floor、character、camera、経路探索
    store/ · hooks/          zustand store、event loop、PTY parser、typewriter
    assets/                  tileset、map、character sheet
docs/                        logo、banner、GitHub Pagesのlanding page
docs/media/                  og.pngとRemotion clip
landing-remotion/            landing page動画用Remotion project
HIVE.md · SPEC.md · DESIGN.md   multi-agent、terminal/event、visual design
docs/message-queue.md        agentのterminalへ入力できる主体と時点
```

素材の出典は[`src/renderer/src/assets/ATTRIBUTION.md`](./src/renderer/src/assets/ATTRIBUTION.md)にあります。

## デザインシステム

画面は**Animal Crossing × Earthbound × SNESのメニューUI**を基調にし、ピクセルへ揃えた大きく親しみやすい部品で構成します。
正本は[`DESIGN.md`](./DESIGN.md)で、すべてのcomponentは同文書のtokenを使います。
ブランド部分にはDunder Mifflinのえんじ色（`#6E1423`）と金色（`#F4D35E`）を重ねています。
15体のアバターは『The Office』の登場人物を、髪、肌、服の組み合わせで描き分けています。

## ロードマップ

v0.4.5までに、12種類のエージェントエンジン、BYOK、ローカルLLM、音声操作、Hive、Kanbanと平日スケジュール、Monaco IDE、外部連携registryとsecret broker、Slack worker、共有hireとAgent Gallery、監視とサーキットブレーカー、永続化、セッション再開、複数window、ワンクリック更新、Skills browser、前提ツール確認、台帳からのcost集計、Apple Siliconで動く意味検索を提供しました。
履歴は[`CHANGELOG.md`](./CHANGELOG.md)にあります。

今後の候補は次のとおりです。

- [ ] **チャット連携の拡充**：TelegramなどからMichaelのキューへ依頼し、返信を戻すbridge。
- [ ] **エンジンと連携templateの追加**：対応エンジンとregistryの拡充。
- [ ] **アバター表現の拡充**：残るstation移動とtool bubbleを実hook eventへ接続。
- [ ] **layoutとcommand履歴の永続化**：agent layoutとsession別履歴へ永続化範囲を拡張。

## 安全、テレメトリー、復旧

公式buildは、アプリ起動、エージェント起動、機能利用という少数の匿名イベントを送信します。
プロンプト、コード、ファイルパス、エージェント出力は送りません。
イベント一覧、匿名性の条件、無効化方法は[`TELEMETRY.md`](./TELEMETRY.md)にあります。

テレメトリーは「設定」の切り替え、`DO_NOT_TRACK`、source buildの三つの方法で無効化できます。
forkには送信用keyが組み込まれないため、sourceからbuildしたforkは何も送信しません。

エージェントがループ、エラー連発、予算超過に入った場合は、サーキットブレーカーで指示変更、制限、停止を選べます。

設定や依存関係の更新後に`node-pty`が読み込めない場合は、破壊的な削除より先に`npm install`を再実行します。
## コントリビューション

このプロジェクトは初期段階のプロトタイプで、コントリビューションを歓迎します。
手順は[`CONTRIBUTING.md`](./CONTRIBUTING.md)にあります。
短い流れは、fork、`npm install && npm run dev`、`npm run typecheck`、[`DESIGN.md`](./DESIGN.md)のtokenを使ったUI実装です。

着手しやすい領域には、実hook eventとの接続、エージェント追加、設定drawer、クロスプラットフォーム対応があります。

> [!IMPORTANT]
> すべてのPull Requestには、変更前と変更後のスクリーンショットを付けます。
> 動きがある場合は録画を使い、PR templateの`### Before`と`### After`へ置いてください。
> 自動検査され、証拠がないPRはmergeされません。
> UI変更がない場合も免除ではなく、示す証拠の種類が変わります。
> 詳細は[Evidence is mandatory](./CONTRIBUTING.md#evidence-is-mandatory)にあります。

質問、bug報告、オフィスの共有は[Discord](https://discord.gg/SEDzP5ZPk5)で受け付けています。
Pull RequestへDiscord handleを書くと、merge後に`employee of the month`roleが付与されます。

## ライセンス

> [!IMPORTANT]
> **素材のライセンスはsource codeと異なります。**
> 同梱するpixel artのtilesetとmapには、[LimeZu](https://limezu.itch.io/moderninteriors)の**Modern Interiors - RPG Tileset [16X16]**を使用しています。
> 商用と非商用の編集および利用を認める**Complete Version licence**が適用され、LimeZuのcreditを残す必要があります。
> 『The Office』の登場人物はLimeZuの素材ではなく、`portraitArt.ts`が手続き的に描画します。
> 詳細は[`src/renderer/src/assets/ATTRIBUTION.md`](./src/renderer/src/assets/ATTRIBUTION.md)にあります。

source codeには[`LICENSE`](./LICENSE)の**MIT License**が適用されます。
MITの許諾範囲はcodeだけです。
同梱するpixel artにはLimeZuの別ライセンスが適用され、`LICENSE`のscope noteでも除外されています。
*Munder Difflin*は愛情を込めたparodyで、NBCの*The Office*およびDunder Mifflinとは関係ありません。

## 謝辞

- [LimeZu](https://limezu.itch.io/)：*Modern Interiors*のpixel-art tileset（Complete Version licence）。
- [`shahar061/the-office`](https://github.com/shahar061/the-office)：office tilesetとmapのvendoring。
- [Pixi.js](https://pixijs.com/)、[xterm.js](https://xtermjs.org/)、[node-pty](https://github.com/microsoft/node-pty)、[electron-vite](https://electron-vite.org/)、[CodeMirror](https://codemirror.net/)：本製品を構成するlibrary。
- [Remotion](https://www.remotion.dev/)：landing pageの「仕組み」動画（`landing-remotion/`）。
- *The Office*（US）：Munder Difflin, Inc.の着想。
