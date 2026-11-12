# Contributing

**English** · [Русский](#русский)

Thanks for wanting to help. Ether is a small project with one maintainer, so the fastest way to
get a change in is to keep it small and say what you tested.

For anything bigger than a bug fix — a new feature, a refactor, a new dependency — open an issue
first. It's cheaper to agree on the shape of a change than to rewrite a pull request.

## Build and run

Rust 1.85 or newer (the crate uses edition 2024), plus the usual Slint build dependencies:

```bash
cargo build
cargo run
cargo test
```

`cargo run` opens the window without touching the network. To actually connect you need
`openvpn` installed and a profile to import; the app starts it through `pkexec`, so you'll get
the normal administrator prompt.

## Where things live

| File | What's in it |
| --- | --- |
| `ui/app.slint` | the whole interface, including the palette at the top |
| `src/main.rs` | window wiring, callbacks, the state the UI shows |
| `src/openvpn.rs` | starting `openvpn`, the management interface, parsing what it says |
| `src/profiles.rs` | importing, storing and renaming `.ovpn` files |
| `src/settings.rs` | `~/.config/ether/settings.json` |
| `src/secrets.rs` | the keyring and the password prompts |
| `src/updater.rs` | checking GitHub for a new release and installing it |
| `src/i18n.rs` | every string in the interface |
| `src/theme.rs`, `tray.rs`, `notify.rs`, `autostart.rs`, `preflight.rs` | the pieces around the edges |

## Translations

Every string lives in `src/i18n.rs` as a row with one column per language. Adding a language is
one more column in that table plus a line in the language picker; nothing else in the code knows
about languages. If you'd rather not touch Rust, send the translated table and it will be wired
up for you.

## Before you send it

```bash
cargo fmt
cargo clippy
cargo test
```

- A behaviour change comes with a test. The existing ones sit next to the code they cover, in
  the same file.
- A UI change comes with a screenshot in the pull request, in both themes if the change touches
  colours.
- One topic per pull request. Unrelated cleanups in the same branch make the diff hard to read.

## Commits

Write the subject in the imperative and say why in the body when it isn't obvious:
`Keep the tray icon after a failed connect`, not `fixed bug`. There's no strict format beyond
that.

The release process lives here instead of the README, so the README can stay user-facing.

## Releasing

Bump the version in `Cargo.toml` and add a `<release>` entry to
`packaging/io.github.etherhaus.EtherOvpn.metainfo.xml`, then push a `v<version>` tag. The Release
workflow builds the tarball, .deb, .rpm, Flatpak and `SHA256SUMS` on Ubuntu 24.04 (so the binary
runs on glibc 2.39 and newer) and attaches them to the GitHub release for the tag, which it
leaves as a draft. Sign the files, upload the `.minisig` ones and publish the release by hand —
the in-app updater doesn't see drafts, so nobody is offered an unsigned update in the meantime.
To write the release notes yourself, create a draft release for the tag before pushing it.

`./packaging/build-packages.sh` builds the same files into `dist/` locally (needs `dpkg-deb`,
`rpmbuild`, `flatpak` and the `org.freedesktop.Platform//25.08` runtime).

### Signing releases

`SHA256SUMS` only proves a download matches the release page. To prove the release page is
ours, sign the files with [minisign](https://jedisct1.github.io/minisign/) and attach the
`.minisig` files to the release.

Create the key once and keep it out of the repository:

```bash
minisign -G                                   # creates ~/.minisign/minisign.key
```

Put the base64 line of `~/.minisign/minisign.pub` (the one after `untrusted comment:`) into
`PUBLIC_KEY` in `src/updater.rs` and rebuild. While `PUBLIC_KEY` is empty Ether skips the
check, so the key has to be in a release before that release's users are protected.

Once the Release workflow has attached the packages to the draft release, sign those files
rather than a local build, or the signatures won't match what was published, then publish it:

```bash
gh release download v<version> -D dist
./packaging/sign-release.sh dist/*            # writes dist/*.minisig
gh release upload v<version> dist/*.minisig
gh release edit v<version> --draft=false --latest
```

`sign-release.sh` refuses to sign with a key that doesn't match `PUBLIC_KEY` and checks every
signature it writes. From then on Ether refuses to install an update whose signature is
missing, unreadable or made with another key, and says which of those it was.

---

## Русский

Спасибо, что хотите помочь. Ether — небольшой проект с одним мейнтейнером, поэтому быстрее
всего изменения попадают внутрь, когда они небольшие и вы рассказали, что проверили.

Если задумали что-то большее, чем исправление бага — новую возможность, рефакторинг, новую
зависимость — сначала откройте issue. Договориться о форме изменения дешевле, чем переписывать
pull request.

### Сборка и запуск

Rust 1.85 или новее (в проекте edition 2024) и обычные зависимости сборки Slint:

```bash
cargo build
cargo run
cargo test
```

`cargo run` открывает окно и не трогает сеть. Чтобы действительно подключиться, нужен
установленный `openvpn` и профиль для импорта; приложение запускает его через `pkexec`, так что
появится обычный запрос прав администратора.

### Что где лежит

| Файл | Что внутри |
| --- | --- |
| `ui/app.slint` | весь интерфейс, включая палитру в начале файла |
| `src/main.rs` | связывание окон, колбэки, состояние, которое показывает интерфейс |
| `src/openvpn.rs` | запуск `openvpn`, management-интерфейс, разбор его вывода |
| `src/profiles.rs` | импорт, хранение и переименование `.ovpn`-файлов |
| `src/settings.rs` | `~/.config/ether/settings.json` |
| `src/secrets.rs` | связка ключей и запросы паролей |
| `src/updater.rs` | проверка новых релизов на GitHub и установка обновлений |
| `src/i18n.rs` | все строки интерфейса |
| `src/theme.rs`, `tray.rs`, `notify.rs`, `autostart.rs`, `preflight.rs` | мелочи по краям |

### Переводы

Каждая строка живёт в `src/i18n.rs` строкой таблицы, где на каждый язык приходится своя колонка.
Добавить язык — это ещё одна колонка в таблице плюс строка в выборе языка; больше нигде в коде
про языки ничего не знают. Если не хочется трогать Rust — пришлите переведённую таблицу, её
подключат за вас.

### Перед отправкой

```bash
cargo fmt
cargo clippy
cargo test
```

- Изменение поведения идёт вместе с тестом. Существующие тесты лежат рядом с кодом, в том же
  файле.
- Изменение интерфейса идёт вместе со скриншотом в pull request — в обеих темах, если затронуты
  цвета.
- Одна тема на pull request. Посторонние уборки в той же ветке делают дифф нечитаемым.

### Коммиты

Тему пишите в повелительном наклонении, а причину — в теле, если она не очевидна:
`Keep the tray icon after a failed connect`, а не `fixed bug`. Строгого формата сверх этого нет.

Процесс выпуска живёт здесь, а не в README, чтобы README оставался пользовательским.

### Выпуск релиза

Поднимите версию в `Cargo.toml` и добавьте запись `<release>` в
`packaging/io.github.etherhaus.EtherOvpn.metainfo.xml`, затем запушьте тег `v<версия>`. Workflow
Release соберёт тарбол, .deb, .rpm, Flatpak и `SHA256SUMS` на Ubuntu 24.04 (чтобы бинарник
работал на glibc 2.39 и новее) и приложит их к релизу на GitHub для этого тега, оставив релиз
черновиком. Подпишите файлы, залейте `.minisig` и опубликуйте релиз руками — встроенный
обновлятор черновики не видит, так что неподписанное обновление никому не предложат. Если хотите
написать заметки к релизу сами, создайте черновик релиза для тега до пуша.

`./packaging/build-packages.sh` собирает те же файлы в `dist/` локально (нужны `dpkg-deb`,
`rpmbuild`, `flatpak` и рантайм `org.freedesktop.Platform//25.08`).

#### Подпись релизов

`SHA256SUMS` доказывает только то, что скачанный файл совпадает со страницей релиза. Чтобы
доказать, что страница релиза наша, подпишите файлы через
[minisign](https://jedisct1.github.io/minisign/) и приложите `.minisig` к релизу.

Ключ создаётся один раз и хранится вне репозитория:

```bash
minisign -G                                   # создаёт ~/.minisign/minisign.key
```

Вставьте base64-строку из `~/.minisign/minisign.pub` (ту, что идёт после `untrusted comment:`)
в `PUBLIC_KEY` в `src/updater.rs` и пересоберите. Пока `PUBLIC_KEY` пуст, Ether проверку
пропускает, поэтому ключ должен попасть в релиз раньше, чем его пользователи окажутся под
защитой.

Когда workflow Release приложил пакеты к черновику релиза, подписывайте именно их, а не
локальную сборку, иначе подписи не совпадут с опубликованными файлами, а затем опубликуйте:

```bash
gh release download v<версия> -D dist
./packaging/sign-release.sh dist/*            # пишет dist/*.minisig
gh release upload v<версия> dist/*.minisig
gh release edit v<версия> --draft=false --latest
```

`sign-release.sh` отказывается подписывать ключом, который не совпадает с `PUBLIC_KEY`, и
проверяет каждую записанную подпись. С этого момента Ether отказывается ставить обновление,
подпись которого отсутствует, не читается или сделана другим ключом, и говорит, что именно
случилось.
