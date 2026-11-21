# batune

Linux(X11/Wayland)と Windows で動く、GPU 描画のネイティブ端末です。

## ビルド

Rust(edition 2024)が要ります。

```sh
cargo run --release
```

Windows 向けは、Linux から mingw でクロスビルドできます。exe へのアイコンの埋め込みには `x86_64-w64-mingw32-windres` を使います(`WINDRES` で場所を変えられます)。

```sh
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

Microsoft Store 向けの MSIX は `scripts/package-msix.sh <batune.exe> <out.msix>` で作ります。

## リリース

`Cargo.toml` の `[package]` の `version` と同じ `v<version>` のタグを push すると、GitHub Actions(`.github/workflows/release.yml`)が Linux x86_64 の `tar.gz` と Windows x86_64 の `zip`、`SHA256SUMS` を作って GitHub Release に添付します。`v0.2.0-rc.1` のように `-` を含むタグはプレリリースになります。

```sh
git tag v0.1.0
git push origin v0.1.0
```

## 使い方

Linux でアプリのランチャーに出すには、`packaging/linux/` の `.desktop` とアイコンを、自分のホームの下に置きます。`batune` は `PATH` の通った場所に置いてください(リポジトリの直下で実行する例です)。

```sh
install -Dm755 target/release/batune ~/.local/bin/batune
install -Dm644 packaging/linux/batune.desktop ~/.local/share/applications/batune.desktop
install -Dm644 packaging/linux/batune.png ~/.local/share/icons/hicolor/128x128/apps/batune.png
```

画像は iTerm2 inline images(OSC 1337)、Sixel、kitty graphics protocol を表示します。kitty は直接転送(`t=d`)だけを受け付け、`a=t`/`T`/`p`/`q`/`d`、分割転送 `m=1`、zlib 圧縮(`o=z`)、`a=d` のすべての `d=`(アニメーションの `f` を除く)に対応します。Unicode placeholder とアニメーションには対応しません。1 枚の上限は 4096×4096 px 相当の画素数で、iTerm2 と kitty の PNG は超えると長辺 4096 px に縮小し、Sixel ははみ出した部分を切ります。保持は表示中・保存中それぞれ 256MB です。

シェルは Linux では `$SHELL`(無ければ `/bin/sh`)、Windows では `pwsh.exe`・`powershell.exe`・`cmd.exe` の順に探します。ウィンドウを閉じると、確認なしですべてのシェルを終了させます。設定ファイルの `[[profiles]]` に名前を付けたシェルを並べておくと、`batune --profile <名前>` でそれを起動できます(後述)。アプリは OSC 0/2 でウィンドウのタイトルを変えられます(`window.dynamic_title = false` で止められます)。

ウィンドウの上端にタブが並びます。+ か Ctrl+Shift+T で既定のシェル(`default_profile`、無ければ `[shell]`)のタブを開き、▾ のメニューからは `[shell]` かプロファイルを選んで開きます。タブはクリックか Ctrl+Tab / Ctrl+Shift+Tab(端まで行くと反対の端に戻ります)で切り替え、× か Ctrl+Shift+W で閉じます。タブの名前は OSC 0/2 のタイトル、無ければプロファイル名(`[shell]` ならプログラム名)です。シェルが終了するとそのタブを閉じ、最後のタブを閉じるとウィンドウを閉じます。タブが多くてもバーはスクロールせず、はみ出した分は見えません。

タブバーの歯車ボタンか Ctrl+, で設定画面を開きます。変えられるのは、既定のプロファイル(`default_profile`)、フォント(名前・大きさ・行の高さ・セルの幅)、テーマと太字を明るい色で描くか、カーソル(形・点滅・点滅の間隔・点滅をやめるまでの秒数)、スクロールバックの行数とホイール 1 目盛りの行数、ウィンドウ(余白・`backdrop`・タイトル・OSC でのタイトル変更)、背景画像の不透明度と合わせ方、画面のベル、OSC 52 の書き込み、入力中にマウスを隠すか、選択したらコピーするかです。「保存」で設定ファイルに書き込みます。変えた項目だけを書き換えるので、コメントや並び順、画面に無い項目(`[[profiles]]`・色の個別指定・キーの割り当てなど)はそのまま残ります。ファイルが無ければ作ります。値が不正なら保存せず、理由を画面に出します。保存した設定は、設定ディレクトリが起動時に無かった場合も含めてすぐに反映されます。反映のされ方はファイルを手で編集したときと同じです(後述)。開いている間のキー入力と IME は設定画面に入り、端末には送りません。Esc か「閉じる」で閉じます(保存していない変更は捨てます)。

マウスのドラッグ・ダブルクリック(単語)・トリプルクリック(行)で選択し、Shift+クリックで選択範囲を延ばします。選択すると既定ですぐクリップボードにコピーします(設定の `[selection]` の `save_to_clipboard = false` で止められます。中クリックでのペーストはありません)。Ctrl+Shift+C でもコピーでき、Ctrl+Shift+V か Shift+Insert でペーストします(Windows では選択中の Ctrl+C でもコピー)。Ctrl+= / Ctrl+- / Ctrl+0 で文字サイズを拡大・縮小・元に戻します。これらのキーは設定ファイルの `[[keyboard.bindings]]` で変えられます。スクロールバックはホイール(既定は 1 ノッチ 3 行)と Shift+PgUp/PgDn(1 画面)で動かします。全画面のプログラム(vim、less など)の中では、ホイールは上下の矢印キーとして、Shift+PgUp/PgDn はそのままプログラムに送られます。マウスを使うプログラム(vim、tmux など)の上では、Shift を押しながら操作すると選択できます。

OSC 52 の書き込み(tmux の `set-clipboard`、vim/neovim の OSC 52 コピーなど)はシステムのクリップボードに入ります(宛先の省略は `c` と同じ。`p`/`s` は Linux の PRIMARY 選択で、Windows では無視する)。1 MiB を超える書き込みは切り詰めずに捨てます。SSH 先や `cat` したファイルもクリップボードを書き換えられるので、不要なら設定ファイルの `[clipboard]` で無効にするか上限を下げてください。読み取りの要求には応答しません。

Ctrl を押しながらマウスを URL や OSC 8 のハイパーリンク(どちらも http/https/mailto だけ)に重ねると下線が付き、Ctrl+クリックで既定のブラウザ・メーラーで開きます。マウスを使うプログラムの上でも Ctrl+クリックはリンクを開く操作になります。

Ctrl+Shift+F で画面とスクロールバックを検索します(大文字・小文字は区別せず、画面上の 1 行の中で探します。折り返した行はまたぎません)。最下行の検索欄に件数が出て、一致した箇所が色付きになります。最初は一番新しい一致に移り、Enter で上(古い出力)へ、Shift+Enter で下へ移り(端まで行くと反対の端に戻ります)、Esc で閉じます。

キー入力は xterm と同じ形で送ります。アプリが kitty keyboard protocol(`CSI > flags u`)を有効にすると、Esc、修飾キー付きの Enter・Tab・Backspace、Ctrl・Alt・Super と組み合わせた文字キーを `CSI code ; mods u` で送ります。Shift+Enter と Enter を区別できるので、Claude Code などでは Shift+Enter で改行できます。対応するのは flag 1(disambiguate)だけで、キーを離したときの通知などほかの flag や、テンキーを別のキーとして送ることには対応しません。

IME で変換中の文字列はカーソル位置に下線付きで表示します。

`ssh` を付けると、ローカルのシェルの代わりに SSH でリモートのシェルを最初のタブに開きます。後から開くタブはローカルのシェルです。ユーザー名を省くと環境変数 `USER`(無ければ `USERNAME`)を使います。ポートの既定は 22 です。

```sh
batune ssh user@example.com
batune ssh user@192.0.2.1:2222
batune ssh user@[2001:db8::1]:2222
```

- **ホスト鍵**: `~/.ssh/known_hosts` と照合します。未知の鍵はウィンドウ内で確認し、`yes` なら追記します。変わった鍵は警告を出し、`yes` と答えたときだけこの接続に限って受け入れます(ファイルは書き換えません)。
- **認証**: 公開鍵 → パスワード(無ければ keyboard-interactive)の順に試します。公開鍵は ssh-agent(Linux は `SSH_AUTH_SOCK`、Windows は `SSH_AUTH_SOCK` か OpenSSH の `\\.\pipe\openssh-ssh-agent`、つながらなければ Pageant)の鍵、次に `~/.ssh/id_ed25519`・`id_ecdsa`・`id_rsa` です。パスフレーズとパスワードはウィンドウ内で入力します(空のパスフレーズでその鍵を飛ばします)。
- 接続や認証に失敗すると、メッセージを表示してキー入力を待ってからタブを閉じます。リモートのシェルが終了するとタブを閉じます。

### 設定ファイル

設定は TOML で、Linux では `$XDG_CONFIG_HOME/batune/config.toml`(既定は `~/.config/batune/config.toml`)、Windows では `%APPDATA%\batune\config.toml` に置きます。ファイルが無ければ既定値で動きます。`batune --print-default-config` で既定値の入ったひな形を出力できます(`-h, --help` で使い方を出します)。

次は設定の例です。既定値とは違います(既定は theme が `batune-dark`、shape が `block`、columns×rows が 80×24)。

```toml
default_profile = "bash"      # 起動するプロファイル(省略すると [shell])。ほかの表より前に書く

[font]
family = "DejaVu Sans Mono"   # 省略すると内蔵の HackGen Console NF
size = 14.0
line_height = 1.0             # セルの高さをフォントの何倍にするか(0.5〜3.0)。文字は真ん中に置く
cell_width = 1.0              # セルの幅をフォントの何倍にするか(0.5〜3.0)

[colors]
theme = "solarized-dark"      # batune-dark / batune-light / solarized-dark / solarized-light / gruvbox-dark
background = "#001e26"        # テーマの色を個別に上書きできる(foreground・cursor・ansi・brights も同様)
bold_is_bright = true         # 太字を ANSI の 0-7 番の色で指定されたら明るい 8-15 番で描く

[cursor]
shape = "bar"                 # block / underline / bar(アプリが DECSCUSR で 2 以上を指定したらそちらが優先)
blinking = false              # 点滅させる(アプリが DECSCUSR の 2/4/6 などで点滅しない形を指定したら止まる)
blink_interval = 750          # 点滅の間隔(ミリ秒)。50〜5000
blink_timeout = 5             # 最後の入力からこの秒数で点滅をやめる(0 で止めない)。86400 まで

[shell]
program = "/bin/bash"         # 省略すると上のとおり $SHELL など
args = ["-l"]
working_directory = "~"

[[profiles]]                  # 名前を付けたシェル。並べた分だけ書く
name = "bash"                 # 必須。重複できない
program = "/bin/bash"         # program・args・working_directory は [shell] と同じ
args = ["-l"]

[window]
columns = 100                 # 起動時の大きさ(文字数)
rows = 30
padding = 6.0                 # グリッドの周りの余白(論理ピクセル)。0〜200
backdrop = "mica"             # Windows 11 のタイトルバー: none(テーマの背景色)/ mica / acrylic
title = "batune"            # ウィンドウのタイトル
dynamic_title = true          # アプリが OSC 0/2 で付けたタイトルを title より優先する
startup_mode = "windowed"     # windowed(columns×rows に合わせる)/ maximized / fullscreen

[background_image]
path = "~/Pictures/wallpaper.png"   # 背景画像(PNG / JPEG / GIF の 1 枚目)。省略すると表示しない
opacity = 0.3                 # テーマの背景色の上に重ねる濃さ(0.0〜1.0)
fit = "cover"                 # cover(全体を覆う)/ contain(全体を収める)/ stretch(引き伸ばす)

[scrolling]
history = 10000               # スクロールバックの行数(0 で無効)
multiplier = 3                # ホイール 1 目盛りでスクロールする行数

[bell]
visual = true                 # BEL で画面を一瞬反転させる(false で何もしない)

[clipboard]
osc52_write = true            # OSC 52 でのクリップボードへの書き込みを許す(false で無視する)
osc52_max_bytes = 1048576     # 書き込める最大バイト数(UTF-8)。超えたものは捨てる。1048576 まで

[selection]
semantic_escape_chars = ",│`|;\"'()[]{}<>"   # ダブルクリックの単語選択で区切りとみなす文字(空白は常に区切り)
save_to_clipboard = true      # 選択したらすぐクリップボードにコピーする

[mouse]
hide_when_typing = false      # キーを打ったらマウスポインターを隠す(動かすとまた出る)

[[keyboard.bindings]]          # キーの割り当て。並べた分だけ書く
key = "F11"
action = "ToggleFullscreen"

[[keyboard.bindings]]
key = "Insert"
mods = "Shift"
action = "None"               # 既定の割り当て(Shift+Insert でペースト)を外してキーをプログラムに送る

[[keyboard.bindings]]
key = "Enter"
mods = "Alt"
chars = "\u001b\r"            # action の代わりに、端末に送る文字列
```

size は 1〜200、line_height/cell_width は 0.5〜3.0、columns/rows は 1 以上、multiplier は 1〜100、padding は 0〜200、blink_interval は 50〜5000、blink_timeout は 0〜86400、osc52_max_bytes は 1048576 以下、opacity は 0.0〜1.0、色は `"#rrggbb"`、ansi/brights は 8 色の配列で書きます。知らないキーや不正な値があるとファイル全体を無視し、理由を標準エラーに出して既定値で起動します。

背景画像の `path` の相対パスは設定ファイルのあるディレクトリから数え、`~` はホームディレクトリです。画像は余白と、背景色がテーマの背景色のままのセルに見え、アプリが背景色を付けたセル(選択範囲や `ls` の色付きの表示など)には隠れます。cover と contain は縦横比を保ち、cover ははみ出した部分を上下か左右で等しく切り、contain は余った部分をテーマの背景色にします。設定ファイルを保存すると画像を読み直します(画像ファイルだけを書き換えても読み直しません)。読めない画像は理由を標準エラーに出し、画像なしで表示します。

起動するシェルは、`--profile <名前>`、`default_profile`、`[shell]` の順に決めます。`--profile` に無い名前を渡すと、プロファイルの一覧を標準エラーに出して終了します。`default_profile` が無い名前を指しているときは、警告を出して `[shell]` で起動します。`[[profiles]]` の `name` が空のときや、同じ `name` を 2 回書いたときは、ほかの不正な値と同じくファイル全体を無視します。`[shell]` だけを書いた設定はこれまでどおり動きます。

Windows では、`wsl.exe -l -q` で見つかった WSL のディストリビューションが、その名前のプロファイル(`wsl.exe -d <名前>`)として自動で加わります(Docker Desktop の `docker-desktop`・`docker-desktop-data` は除きます)。`batune --profile Ubuntu` や `default_profile = "Ubuntu"` で WSL を起動できます。同じ名前の `[[profiles]]` を書くとそちらを使うので、`args = ["-d", "Ubuntu", "--cd", "~"]` のように起動の仕方を変えられます。`wsl.exe` は最初に必要になったとき(ふつうは起動時)に裏で 1 回だけ呼び、終わるまではタブのメニューと設定画面に WSL のディストリビューションが出ません。`--profile` や `default_profile` が設定に無い名前のときだけ、その結果を待ちます。

`[[keyboard.bindings]]` は次のように書きます。利用者の割り当ては既定の割り当てより優先し、同じキーと修飾キーの組を何度も書いたら後のものが効きます。

- `key`: 1 文字(`"C"`、`"0"`、`"="` など。英字の大文字・小文字は区別しません)か、`PageUp` `PageDown` `Home` `End` `Insert` `Delete` `Up` `Down` `Left` `Right` `Tab` `Enter` `Escape` `Backspace` `Space` `F1`〜`F12` `Plus` `Minus` `Equals`
- `mods`: `Control`・`Shift`・`Alt`・`Super` を `|` でつなぎます(`"Control|Shift"`)。省くと修飾キーなしです。修飾キーは完全に一致したときだけ効きます
- `mode`: `"Selection"` と書くと、文字を選択しているときだけ効きます
- `action` か `chars` のどちらか一方: `chars` は端末に送る文字列です。`action` は次のどれかです

| action | 既定のキー | 動き |
|---|---|---|
| `Copy` | Ctrl+Shift+C(Windows では選択中の Ctrl+C も) | 選択した文字列をコピー |
| `Paste` | Ctrl+Shift+V、Shift+Insert | ペースト |
| `Search` | Ctrl+Shift+F | スクロールバック検索を開く |
| `ScrollPageUp` / `ScrollPageDown` | Shift+PgUp / Shift+PgDn | 1 画面スクロール |
| `ScrollLineUp` / `ScrollLineDown` | — | 1 行スクロール |
| `ScrollToTop` / `ScrollToBottom` | — | スクロールバックの先頭 / 末尾(今の画面)へ |
| `IncreaseFontSize` / `DecreaseFontSize` | Ctrl+= と Ctrl++(Ctrl+Shift++ も) / Ctrl+- | 文字サイズを 1 ポイントずつ変える(1〜200) |
| `ResetFontSize` | Ctrl+0 | 文字サイズを `font.size` に戻す |
| `ToggleFullscreen` | — | 全画面の切り替え |
| `ClearSelection` | — | 選択を解除する(何も選択していなければキーをそのまま送る) |
| `NewTab` | Ctrl+Shift+T | 既定のシェルのタブを開く |
| `CloseTab` | Ctrl+Shift+W | 今のタブを閉じる |
| `NextTab` / `PreviousTab` | Ctrl+Tab / Ctrl+Shift+Tab | 右 / 左のタブへ |
| `OpenSettings` | Ctrl+, | 設定画面を開く |
| `None` | — | 既定の割り当てを外し、キーをプログラムに送る |

スクロールの action は、全画面のプログラム(vim、less など)の中ではキーをそのままプログラムに送ります。文字サイズの変更はそのウィンドウの実行中だけのもので、設定ファイルの `font.size` は書き換えません。知らない key・mods・mode・action や、`action` と `chars` の両方を書いた割り当ては、ほかの設定と同じく不正な値として扱います。

Windows 11 ではタイトルバーをテーマの背景色と文字色にし、角を丸めます。タイトルバーのボタンは背景が暗ければダークになります(Windows 10 ではこれだけが効きます)。`backdrop` の mica / acrylic は Windows 11 22H2 以降で効き、タイトルバーにだけかかります(端末の部分は透けません)。どちらも設定の保存でその場で切り替わります。

起動中にファイルを保存すると、フォント・配色・カーソル・余白・スクロール・ベル・クリップボード・選択・マウス・ウィンドウのタイトル・キーの割り当てはその場ですべてのタブに反映されます(`[shell]`・`[[profiles]]`・`default_profile` は次に開くタブから、`window.columns`/`rows`/`startup_mode` は次の起動から)。保存したファイルが不正なときは、理由を標準エラーに出して直前の設定のままにします。監視するのは起動時に設定ディレクトリ(`batune/`)があった場合だけです(ファイルは後から作っても反映されます。消したり名前を変えたりしても今の設定のままです)。設定ディレクトリが無かった場合も、設定画面から保存した設定は反映されますが、手で編集した変更は次の起動からです。


## 開発

```sh
cargo test
```

| モジュール | 役割 |
|---|---|
| `src/vt` | OS と描画に依存しない端末エミュレーションの中核(グリッド・パーサ・制御シーケンス) |
| `src/render` | wgpu によるセルグリッドの描画 |
| `src/pty` | Linux の PTY / Windows の ConPTY |
| `src/app` ほか `src/` 直下 | ウィンドウ・タブ・設定・SSH などのアプリ本体(`src/main.rs` がバイナリ `batune`) |

## ライセンス

MIT。同梱のフォント HackGen Console NF は `fonts/LICENSE` に従います。
