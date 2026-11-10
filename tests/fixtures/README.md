# Synthetic native acceptance audio

`synthetic-ru.wav` is an offline, computer-generated Russian speech fixture:

> Это синтетическая тестовая запись. Проверяем импорт, распознавание и сохранение текста.

It was generated with the built-in Windows `System.Speech` Russian synthesizer,
without recording a microphone or playing audio. It contains no meeting or
personal recording. The adjacent JSON records its exact format, duration and
SHA-256. It is distributed under this repository's MIT license.

The native CI journey imports this actual file, runs the default local model,
requires at least five of six specified content keywords and valid segment
times, then exports every recognized segment to DOCX, Markdown, text and SRT.
This short fixture checks the supported execution path; it is not a general
speech-recognition accuracy benchmark.
