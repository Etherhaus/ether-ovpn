# Security policy

**English** · [Русский](#русский)

## Supported versions

Only the [latest release](https://github.com/Etherhaus/ether-ovpn/releases/latest) gets fixes.
There are no long-lived branches: when something is fixed, it is fixed in a new release.

## Reporting a vulnerability

Please don't open a public issue. Use GitHub's private form:
[**Report a vulnerability**](https://github.com/Etherhaus/ether-ovpn/security/advisories/new).
If that doesn't work for you, write to <Lehzenblad@proton.me>.

A useful report says what you did, what happened, and which version, distribution and install
format you used. If the app is involved, the **Copy diagnostics** button in the log window puts
the version, the OS, the OpenVPN version and the last 50 log lines on your clipboard.

I'm one person, so expect a first reply in a few days rather than in a few hours. If the report
holds up, the fix goes into the next release and the advisory is published with credit, unless
you'd rather stay anonymous.

## What is worth reporting

- Anything that lets a profile, a server or a downloaded update run code, read files or reach
  the network without the user asking for it.
- Escaping the privilege boundary. Ether runs as your user and starts `openvpn` through
  `pkexec`; a way to get root without the Polkit prompt is a vulnerability.
- Leaks of profile contents, private keys or passwords — into logs, the clipboard, the config
  directory or a crash report.
- The management interface. It listens on a random localhost port with a one-time password;
  another local user reaching it is a bug.

## What is not

- OpenVPN itself, its configuration or your server. Report those upstream.
- The Flatpak having host access. That is by design: OpenVPN has to run as root on the host, so
  the sandbox is not a security boundary here.
- The update signature check being off. `PUBLIC_KEY` in `src/updater.rs` is empty, so updates
  are verified by HTTPS and `SHA256SUMS` only. It is a known gap, not a surprise, and it closes
  when the releases get signed.
- A profile that carries `up`, `down` or `plugin` directives. Ether warns about them before
  connecting instead of refusing to import.

---

## Русский

### Поддерживаемые версии

Исправления выходят только для [последнего релиза](https://github.com/Etherhaus/ether-ovpn/releases/latest).
Долгоживущих веток нет: если что-то исправлено, оно исправлено в новом релизе.

### Как сообщить об уязвимости

Не открывайте публичный issue. Воспользуйтесь приватной формой GitHub:
[**Report a vulnerability**](https://github.com/Etherhaus/ether-ovpn/security/advisories/new).
Если она недоступна — напишите на <Lehzenblad@proton.me>.

В хорошем отчёте есть шаги, что произошло, и версия с дистрибутивом и форматом установки.
Если дело в приложении, кнопка **Скопировать диагностику** в окне лога положит в буфер версию,
ОС, версию OpenVPN и последние 50 строк лога.

Я один человек, поэтому первого ответа стоит ждать через несколько дней, а не через час. Если
отчёт подтвердится, исправление войдёт в следующий релиз, а advisory опубликуют с указанием
авторства — если только вы не предпочтёте остаться анонимным.

### Что стоит присылать

- Всё, что позволяет профилю, серверу или скачанному обновлению выполнить код, прочитать файлы
  или выйти в сеть без ведома пользователя.
- Выход за границу прав. Ether работает от вашего пользователя и запускает `openvpn` через
  `pkexec`; способ получить root без запроса Polkit — это уязвимость.
- Утечку содержимого профилей, приватных ключей или паролей — в логи, буфер обмена, каталог
  настроек или отчёт о падении.
- Management-интерфейс. Он слушает случайный порт localhost с одноразовым паролем; если до
  него дотянется другой локальный пользователь — это баг.

### Что не стоит

- Сам OpenVPN, его конфигурация и ваш сервер — это к апстриму.
- Доступ Flatpak к хосту. Так и задумано: OpenVPN должен работать от root на хосте, поэтому
  песочница здесь не граница безопасности.
- Выключенная проверка подписи обновлений. `PUBLIC_KEY` в `src/updater.rs` пуст, обновления
  проверяются только по HTTPS и `SHA256SUMS`. Это известный пробел, а не сюрприз; он закроется,
  когда релизы начнут подписывать.
- Профиль с директивами `up`, `down` или `plugin`. Ether предупреждает о них перед
  подключением, а не отказывается импортировать.
