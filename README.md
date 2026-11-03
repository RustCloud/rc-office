# rc-office — «Офис» для RustCloud

Плагин RustCloud `rc-office` (в панели — «Офис»): документы, таблицы и презентации из файлов
пользователя (`.docx`, `.xlsx`, `.pptx`, `.odt`, `.ods`, `.odp`, `.doc`, `.xls`, `.ppt`, `.rtf`, `.csv`…)
открываются и редактируются в [Collabora Online](https://www.collaboraonline.com/). Collabora —
WOPI-клиент в отдельном контейнере, плагин — WOPI-сервер поверх файлового API ядра. Документ остаётся
обычным файлом пользователя: версии, квота, журнал и корзина работают как у любого другого файла.

- **Открыть** — в «Файлах» пункт «Редактировать в Офисе» в меню документа или двойной щелчок по нему.
  Редактор открывается на всё окно; «×» панели или «Закрыть» в Collabora возвращают к файлам.
- **Сохранение** — Collabora сохраняет сама (автосохранение и при закрытии); каждое сохранение — новая
  [версия](https://github.com/RustCloud/rustcloud) файла в окне «Версии» (сохранения одного человека
  чаще раза в 5 минут ядро схлопывает в одну).
- **Блокировка** — пока документ открыт на правку, в списке файлов на нём замок: панель и другие
  приложения его не меняют. Закрыли редактор — замок снимается; Collabora пропала — замок истекает
  сам через 30 минут (или «Снять блокировку» в панели).
- **Конфликт** — если файл изменился в облаке, пока открыт редактор, Collabora спросит, перезаписать
  ли его или перезагрузить документ: чужие правки молча не затираются.
- **Только чтение** — файл, к которому у пользователя нет права записи (общий только на чтение), и
  форматы, которые Collabora лишь показывает (PDF), открываются для чтения. Если документ сейчас
  держит другое приложение, «Офис» так и скажет и предложит открыть его для чтения.
- **Большие файлы** — документ не проходит через память плагина: ядро отдаёт его Collabora потоком
  (`x-rustcloud-send`) и потоком же сохраняет (`files.write-request-body`, `large_bodies`).

## Установка

1. Запустите Collabora Online (CODE) рядом с облаком, например в том же `compose.yml`:

   ```yaml
   services:
     collabora:
       image: collabora/code:latest
       ports:
         - "9980:9980"
       environment:
         # Адрес облака, с которого Collabora принимает документы (WOPISrc): точно как его видит
         # Collabora, со схемой и портом. Несколько — aliasgroup2, aliasgroup3…
         aliasgroup1: "https://cloud.example.com:443"
         # За HTTPS-прокси: TLS снимает прокси, Collabora — без TLS.
         extra_params: "--o:ssl.enable=false --o:ssl.termination=true"
       cap_add:
         - MKNOD
       restart: unless-stopped
   ```

   Образ ~1,8 ГБ, без открытых документов Collabora занимает около 770 МБ памяти.
   Без прокси и без HTTPS (стенд в локальной сети) — `--o:ssl.enable=false` без `ssl.termination`.

2. Установите `rc-office.zip` в «Администрирование → Плагины» (архив — в
   [релизах](https://github.com/RustCloud/rc-office/releases) или артефакт сборки Actions) и включите
   плагин: это подтверждает доступ к открытому файлу и адрес Collabora в `network` и `frame`.

3. Заполните настройки плагина там же:

   | Настройка        | Что это                                                                   |
   |------------------|---------------------------------------------------------------------------|
   | `office_url`     | адрес Collabora, по которому до неё достают **и браузеры, и сервер облака**: `https://office.example.com` |
   | `wopi_base_url`  | адрес облака для Collabora, если он не совпадает с адресом панели — например, в compose-сети: `http://rustcloud:8080`. Пусто — адрес панели (`context.origin`) |

   С `wopi_base_url` (или адресом панели) должен совпадать `aliasgroup1` Collabora — иначе она
   откажется открывать документы («Unauthorized WOPI host»).

4. Страница «Офис» в панели показывает, отвечает ли Collabora, её версию и форматы, которые она
   редактирует.

**HTTPS со своим ЦС.** Если облако за HTTPS с сертификатом своего ЦС, Collabora нужен этот ЦС внутри
контейнера (иначе она не достучится до WOPI): смонтируйте его в
`/usr/local/share/ca-certificates/` и пересоберите доверенные сертификаты при запуске, либо дайте
Collabora адрес облака по HTTP внутри сети через `wopi_base_url`.

**Встраивание.** Панель показывает редактор во фрейме, поэтому Collabora должна разрешать встраивание
в страницы облака: по умолчанию она разрешает хосты из `aliasgroup1`; если адрес панели другой —
`--o:net.frame_ancestors=https://cloud.example.com`.

## Как устроен

| Где                                              | Что                                                          |
|--------------------------------------------------|--------------------------------------------------------------|
| [src/wopi/](src/wopi)                            | WOPI: CheckFileInfo, GetFile, PutFile, блокировки            |
| [src/session.rs](src/session.rs)                 | `/session` — адрес редактора и токен для страницы; `/status` |
| [src/discovery.rs](src/discovery.rs)             | discovery Collabora: разбор, кэш в хранилище плагина на час, `urlsrc` |
| [src/backend.rs](src/backend.rs)                 | трейт над импортами ядра — логика тестируется без WASM       |
| [src/host.rs](src/host.rs)                       | этот трейт поверх импортов WIT                               |
| [ui/](ui)                                        | страницы: `/edit` (редактор) и `/` (состояние связи)         |

```text
«Редактировать в Офисе» → панель: доступ session к файлу, #/app/rc-office/edit?file=<id>
страница /edit ── /session (через мост) ──▶ плагин: discovery (кэш), urlsrc по расширению,
                                             tokens.issue(file, write, 10 ч), origin панели → AppData
rustcloud.openFrame(url, { access_token, access_token_ttl }) → панель показывает Collabora поверх страницы
Collabora ── ?access_token=… ──▶ /app/rc-office/wopi/files/<id>[/contents]
```

- **Токен** — на один файл, на 10 часов (у ядра до 12): после этого Collabora попросит перезагрузить
  документ. Работает, пока у плагина есть доступ к файлу: ядро держит доступ `session`, пока
  открыта страница, и ещё минуту — чтобы Collabora успела сохранить.
- **WOPI-запросы** принимаются только с токеном именно этого файла (`tokens.current()`); маршруты
  страницы (`/session`, `/status`) — только от пользователя, не по токену.
- **PutFile**: файл заблокирован — `X-WOPI-Lock` должен совпасть (иначе `409` с текущим); без
  блокировки пишется только пустой файл (так требует WOPI); `X-COOL-WOPI-Timestamp` ≠ времени файла —
  `409` с `COOLStatusCode: 1010`, и Collabora спрашивает пользователя; запись — с `If-Match` по ETag
  того же момента, так что запись, которую обогнали, тоже получает `1010`.
- **Блокировки** — блокировки ядра (до часа; WOPI просит 30 минут): `LOCK`, `REFRESH_LOCK`, `UNLOCK`,
  `GET_LOCK`, `LOCK` с `X-WOPI-OldLock` (UnlockAndRelock). Блокировка другого приложения — `409` с
  пустым `X-WOPI-Lock` и `X-WOPI-LockFailureReason`.
- **PostMessageOrigin** — адрес панели: страница передаёт `context.origin` в `/session`, плагин
  хранит его в AppData пользователя (`sessions/<id>.json`) до CheckFileInfo.
- Пока нет: «Сохранить как» и переименование (`PUT_RELATIVE`, `RENAME_FILE` — `501`), создание
  документов из шаблонов, превью и PDF, proof keys — см. [PLAN.md](PLAN.md).

## Сборка

```sh
rustup target add wasm32-wasip2
cargo test
cargo build --release --target wasm32-wasip2
mkdir -p dist/rc-office && cp manifest.toml dist/rc-office/
cp target/wasm32-wasip2/release/rc_office.wasm dist/rc-office/app.wasm
(cd dist && zip -r rc-office.zip rc-office)
```

[.github/workflows/build.yml](.github/workflows/build.yml) делает то же на каждый пуш (проверка версий,
`fmt`, `clippy`, тесты, JS страниц) и выкладывает артефакт `rc-office-<версия>` — его zip ставится как
есть. Тег `v<версия>` (версия из `manifest.toml`) публикует релиз с `rc-office.zip`. Версия плагина
растёт с каждым пушем — в `manifest.toml` и `Cargo.toml` одинаково.

`wit/app.wit` — копия `crates/plugin-api/wit/app.wit` ядра версии из `manifest.toml` (`api`).
