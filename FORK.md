# herdr-orca

Форк [herdr](https://github.com/herdrdev/herdr) с левой панелью в духе
[Orca](https://github.com/stablyai/orca). herdr остаётся терминальным
приложением; меняется только боковая панель, и только если её включить.

Ветка `orca-main` — наши изменения поверх `master` апстрима.

## Что добавлено

### Карточная боковая панель (`[ui.sidebar] layout = "cards"`)

Вместо двух блоков «spaces» и «agents» — один список карточек рабочих
пространств. Внутри карточки — дерево: вкладки → панели, агенты прямо в дереве
со статусом и заголовком.

```
 workspaces              status
 ⌕ filter
 ▾ × NEEDS INPUT · 1
 ▎× herdr-orca
 ▎  ⎇ feat/cards ↑2
 ▎  ├ claude
 ▎  │  └ × claude blocked
 ▎  │       fix sidebar
 ▎  └ build
 ▎     ├ › cargo
 ▎     └ › zsh

 ▾ ◐ WORKING · 1
 ▎◐ bitrix-modules
 ▎  └ ◐ claude working
 ───────────────────────────
  5h 92% (1h30m) · 7d 94% (6d9h)
 new                   menu «
```

- **Секции по статусу**: ждёт ответа → готово → работает → простаивает.
  Клик по заголовку секции сворачивает её (фокусная карточка остаётся видна).
  Состояние запоминается.
- **Группировка**: клик по `status` в шапке переключает
  `status → project → flat` (по репозиторию / плоский список). Запоминается.
- **Закреплённые (Pinned)**: ПКМ по карточке → `Pin` / `Unpin`. Закреплённые
  всегда сверху, в порядке закрепления.
- **Фильтр**: клик по строке `⌕ filter` или `/` в режиме навигации
  (`prefix` + `w`). Ищет по имени, ветке, репозиторию, вкладкам, панелям,
  агентам и их заголовкам. `Enter` — оставить фильтр, `Esc` — сбросить.
- **Лимиты внизу панели**: `footer_command` — любая команда, первая строка её
  вывода показывается внизу (например, `herdr-limits.sh` из herder-botox).
- Клик по карточке — перейти в пространство, по вкладке — во вкладку, по
  панели/агенту — в панель. ПКМ по вкладке/панели — их обычные меню.
- `↑/↓` в режиме навигации и переключение пространств идут в порядке карточек.

Свёрнутая панель, мобильная раскладка и режим с несколькими машинами остаются
прежними. Перетаскивание пространств в карточном режиме отключено — порядок
задают секции.

Настройки:

```toml
[ui.sidebar]
layout = "cards"            # "panels" — как в оригинальном herdr

[ui.sidebar.cards]
group_by = "status"         # status | project | none
card_gap = 1                # пустых строк между карточками
# Строки агента внутри карточки — те же токены, что в [ui.sidebar.agents].
agent_rows = [["state_icon", "agent", "state_text"], ["terminal_title_stripped"]]
footer_command = "$HOME/herder-botox/bin/herdr-limits.sh"
footer_interval_seconds = 30

# Подписи секций (ключи: pinned, blocked, done, working, idle, no_repo, rest)
[ui.sidebar.cards.section_labels]
pinned = "закреплённые"
blocked = "ждёт ответа"
done = "готово"
working = "работает"
idle = "простаивает"
```

Строки самой карточки (имя, ветка) берутся из `[ui.sidebar.spaces].rows`.
Готовый фрагмент: [`contrib/herder-botox/config/herdr-orca-cards.toml`](contrib/herder-botox/config/herdr-orca-cards.toml).

### contrib/herder-botox

Наши скрипты и конфиг для herdr (статус агентов, контекст, лимиты, прокси,
копирование имени вкладки). См. [`contrib/herder-botox/README.md`](contrib/herder-botox/README.md).
Работают и со стоковым herdr.

## Сборка

Нужны Rust из `rust-toolchain.toml` и Zig 0.16.0 (для вендорного libghostty-vt).

```sh
ZIG=/path/to/zig-0.16.0/zig cargo build --release
cp target/release/herdr ~/.local/bin/herdr   # заменить установленный
herdr server stop                            # чтобы сервер поднялся новым бинарём
```

## Обновление из апстрима

```sh
git remote add upstream https://github.com/herdrdev/herdr   # один раз
git fetch upstream
git rebase upstream/master orca-main
```
