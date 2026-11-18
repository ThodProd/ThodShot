# Совместимость с ThodHUB

- Исполняемый файл `ThodShot.exe` поддерживает `--register-integrations`, `--cleanup`, `--stop-background`, `--restart`, `--enable-printscreen`, `--disable-printscreen`, `--background`, `--capture` и `--settings`.
- Настройки и журнал находятся в `%LOCALAPPDATA%\ThodHUB\Data\thodshot` или в каталоге `THODHUB_DATA_DIR`. Путь из ThodHUB сохраняется в `%LOCALAPPDATA%\ThodShot\datadir.txt` для последующих запусков из трея и автозагрузки.
- Файл настроек, запись автозапуска, сохранённая привязка PrintScreen и имена объектов IPC совместимы с версией 1.1.9. При удалении интеграций Windows получает прежнюю привязку PrintScreen.
- Захват использует Windows Graphics Capture и переключается на GDI BitBlt, если WGC недоступен или вернул пустой кадр. При выборе «Весь экран» сохраняется монитор под курсором. В режиме «Область» можно начать выделение на одном мониторе и закончить на другом.
- Исходный код версии на C# доступен по тегу `hub-v1.1.9`.
