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
