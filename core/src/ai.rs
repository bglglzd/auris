use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::transcript::{speaker_label, Transcript};

/// Настройки доступа к OpenAI-совместимому ИИ.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// Предложение метаданных встречи от ИИ.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MetadataSuggestion {
    #[serde(default, deserialize_with = "string_or_seq")]
    pub title: String,
    #[serde(default, deserialize_with = "string_or_seq")]
    pub participants: String,
    #[serde(default, deserialize_with = "string_or_seq")]
    pub topic: String,
}

/// Модель иногда возвращает поле массивом (напр. participants: ["A","B"]) или
/// числом вместо строки — приводим к строке (массив → через запятую). Иначе
/// serde падает «invalid type: sequence, expected a string».
fn string_or_seq<'de, D>(de: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(de)?;
    Ok(match v {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(a) => a
            .into_iter()
            .map(|x| match x {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    })
}

/// Абстракция «спросить чат-модель»: даёт system+user, получает текст ответа.
pub trait ChatBackend: Send + Sync {
    fn chat(&self, system: &str, user: &str) -> AppResult<String>;
}

/// Реальный бэкенд: HTTP к {base_url}/chat/completions (OpenAI-совместимый).
///
/// Модель на сервере пользователя может меняться (выкатили новую версию —
/// сменилось имя). Поэтому: пустая модель или «auto» — берём ту, что сервер
/// отдаёт в `/models`; если сервер ответил «такой модели нет» — один раз
/// перечитываем список, выбираем ближайшую актуальную и повторяем запрос.
pub struct HttpChatBackend {
    config: AiConfig,
    /// Модель, которой реально отвечает сервер (после авто-выбора/подмены).
    model: std::sync::Mutex<Option<String>>,
}

/// Модели, которые отдаёт OpenAI-совместимый сервер (`GET {base}/models`).
/// Понимает `{"data":[{"id":…}]}` (OpenAI, vLLM, llama.cpp, LM Studio) и
/// `{"models":[{"name"|"model":…}]}` (Ollama и новые llama.cpp).
pub fn list_models(config: &AiConfig) -> AppResult<Vec<String>> {
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", config.api_key))
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(http_err)?;
    let v: serde_json::Value = resp.into_json().map_err(|e| AppError::Http(e.to_string()))?;
    let mut out: Vec<String> = Vec::new();
    for key in ["data", "models"] {
        if let Some(arr) = v.get(key).and_then(|x| x.as_array()) {
            for m in arr {
                let id = m
                    .get("id")
                    .or_else(|| m.get("model"))
                    .or_else(|| m.get("name"))
                    .and_then(|x| x.as_str());
                if let Some(id) = id {
                    if !out.iter().any(|o| o == id) {
                        out.push(id.to_string());
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Какую модель использовать: настроенную, если сервер её отдаёт; иначе —
/// ближайшую по имени (самый длинный общий префикс: «qwen3.5-27b» →
/// «qwen3.6-27b»), а без совпадений — первую. `None` — сервер пуст.
pub fn pick_model(configured: &str, available: &[String]) -> Option<String> {
    let want = configured.trim();
    if available.is_empty() {
        return None;
    }
    if !want.is_empty() && want != "auto" {
        if let Some(m) = available.iter().find(|m| m.as_str() == want) {
            return Some(m.clone());
        }
        let common = |a: &str, b: &str| {
            a.chars()
                .zip(b.chars())
                .take_while(|(x, y)| x.eq_ignore_ascii_case(y))
                .count()
        };
        let best = available
            .iter()
            .map(|m| (common(want, m), m))
            .max_by_key(|(n, _)| *n)
            .filter(|(n, _)| *n >= 3)
            .map(|(_, m)| m.clone());
        if best.is_some() {
            return best;
        }
    }
    Some(available[0].clone())
}

/// Результат проверки подключения к ИИ (для настроек).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AiCheck {
    pub ok: bool,
    /// Модели на сервере.
    pub models: Vec<String>,
    /// Какую модель стоит использовать (актуальная на сервере).
    pub model: String,
    /// Настроенной модели на сервере больше нет — выбрана другая.
    pub changed: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Проверяет сервер: доступен ли, какие модели отдаёт и какую использовать.
pub fn check(config: &AiConfig) -> AiCheck {
    let t = std::time::Instant::now();
    match list_models(config) {
        Ok(models) => {
            let model = pick_model(&config.model, &models).unwrap_or_else(|| config.model.clone());
            let want = config.model.trim();
            let changed = !models.is_empty() && !want.is_empty() && want != "auto" && model != want;
            AiCheck {
                ok: true,
                models,
                model,
                changed,
                latency_ms: t.elapsed().as_millis() as u64,
                error: None,
            }
        }
        Err(e) => AiCheck {
            ok: false,
            model: config.model.clone(),
            latency_ms: t.elapsed().as_millis() as u64,
            error: Some(e.to_string()),
            ..Default::default()
        },
    }
}

/// Ошибка HTTP с телом ответа (сервер обычно объясняет, что не так).
fn http_err(e: ureq::Error) -> AppError {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            AppError::Http(format!("HTTP {code}: {}", body.chars().take(400).collect::<String>()))
        }
        other => AppError::Http(other.to_string()),
    }
}

/// Похоже ли на «такой модели на сервере нет».
fn is_model_missing(e: &AppError) -> bool {
    let s = e.to_string().to_lowercase();
    (s.contains("http 404") || s.contains("http 400") || s.contains("http 422"))
        && s.contains("model")
}

/// Убирает блок рассуждений `<think>…</think>` reasoning-моделей (Qwen3,
/// DeepSeek-R1 и др.) — пользователю нужен только ответ.
pub fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    out.trim().to_string()
}

impl HttpChatBackend {
    pub fn new(config: AiConfig) -> Self {
        Self { config, model: std::sync::Mutex::new(None) }
    }

    /// Модель, которой реально шли запросы (после авто-выбора), если менялась.
    pub fn used_model(&self) -> Option<String> {
        self.model.lock().unwrap().clone()
    }

    fn current_model(&self) -> AppResult<String> {
        if let Some(m) = self.model.lock().unwrap().clone() {
            return Ok(m);
        }
        let want = self.config.model.trim();
        if !want.is_empty() && want != "auto" {
            return Ok(want.to_string());
        }
        // Модель не задана — берём ту, что отдаёт сервер.
        let models = list_models(&self.config)?;
        let m = pick_model(want, &models)
            .ok_or_else(|| AppError::Http("сервер ИИ не отдаёт ни одной модели".into()))?;
        *self.model.lock().unwrap() = Some(m.clone());
        Ok(m)
    }

    fn post(&self, model: &str, system: &str, user: &str) -> AppResult<String> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        });
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(http_err)?;
        let value: serde_json::Value =
            resp.into_json().map_err(|e| AppError::Http(e.to_string()))?;
        let content = value
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| AppError::Http("unexpected response shape".into()))?;
        Ok(strip_think(content))
    }
}

impl ChatBackend for HttpChatBackend {
    fn chat(&self, system: &str, user: &str) -> AppResult<String> {
        let model = self.current_model()?;
        match self.post(&model, system, user) {
            Err(e) if is_model_missing(&e) => {
                // Модель на сервере сменилась — берём актуальную и повторяем.
                let models = list_models(&self.config).map_err(|_| e)?;
                let next = match pick_model(&model, &models) {
                    Some(m) if m != model => m,
                    _ => return Err(AppError::Http(format!("модель «{model}» недоступна на сервере"))),
                };
                *self.model.lock().unwrap() = Some(next.clone());
                self.post(&next, system, user)
            }
            other => other,
        }
    }
}

/// Превращает расшифровку в простой текст для подачи модели.
pub fn transcript_to_text(transcript: &Transcript) -> String {
    transcript
        .segments
        .iter()
        .map(|s| format!("{}: {}", speaker_label(&s.speaker), s.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Вытаскивает первый JSON-объект из текста (модель может добавить пояснения
/// или обернуть в ```-блок).
fn extract_json(s: &str) -> &str {
    match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b >= a => &s[a..=b],
        _ => s,
    }
}

/// Просит модель предложить заголовок/участников/тему по расшифровке.
pub fn suggest_metadata(
    backend: &dyn ChatBackend,
    transcript_text: &str,
) -> AppResult<MetadataSuggestion> {
    let system = "Ты анализируешь расшифровку разговора и возвращаешь СТРОГО JSON с полями \
        title (короткий заголовок встречи), participants (участники через запятую), \
        topic (тема одним предложением). Только JSON, без пояснений.";
    let content = backend.chat(system, transcript_text)?;
    let json = extract_json(&content);
    let suggestion: MetadataSuggestion = serde_json::from_str(json)?;
    Ok(suggestion)
}

/// Краткая выжимка короткого разговора (один проход).
pub fn summarize(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    let system = "Сделай структурированную выжимку разговора в формате Markdown: \
        заголовки (##), маркированные списки, выделение важного. Разделы: краткое \
        резюме, ключевые темы, решения, задачи и договорённости. По-русски, по делу.";
    backend.chat(system, transcript_text)
}

/// Порог длины (в символах) одного фрагмента для длинной выжимки.
/// Подобрано консервативно под модели с контекстом ~130k токенов.
pub const SUMMARY_CHUNK_CHARS: usize = 60_000;

/// Разбивает текст на части не длиннее `max` символов по границам строк.
fn split_chunks(text: &str, max: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if !cur.is_empty() && cur.len() + line.len() + 1 > max {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
        if cur.len() >= max {
            chunks.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// Выжимка разговора любой длины. Короткий — один проход; длинный (не влезает
/// в контекст модели) — map-reduce: суммируем каждый фрагмент, затем объединяем
/// конспекты (рекурсивно, если их сумма всё ещё велика).
pub fn summarize_long(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    if transcript_text.len() <= SUMMARY_CHUNK_CHARS {
        return summarize(backend, transcript_text);
    }

    let chunks = split_chunks(transcript_text, SUMMARY_CHUNK_CHARS);
    let total = chunks.len();
    let mut partials = Vec::with_capacity(total);
    for (i, chunk) in chunks.iter().enumerate() {
        let system = "Ты конспектируешь ОДНУ ЧАСТЬ длинного разговора. Кратко и по делу \
            изложи ключевые моменты, решения и задачи именно из этого фрагмента. \
            По-русски. Без вступлений.";
        let user = format!("Часть {}/{} разговора:\n\n{}", i + 1, total, chunk);
        partials.push(backend.chat(system, &user)?);
    }

    let combined = partials.join("\n\n---\n\n");
    // Если конспектов всё ещё слишком много — сворачиваем ещё раз.
    if combined.len() > SUMMARY_CHUNK_CHARS {
        return summarize_long(backend, &combined);
    }

    let system = "Объедини конспекты частей одного разговора в единую связную итоговую \
        выжимку в формате Markdown: заголовки (##), списки, выделение важного. \
        Разделы: краткое резюме, ключевые темы, решения, задачи и договорённости. \
        Не повторяйся, убери дубли. По-русски.";
    backend.chat(system, &combined)
}

/// Краткое резюме (TL;DR) — несколько предложений (один проход).
pub fn brief_summary(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    let system = "Дай очень краткое резюме разговора: 2–4 предложения о том, о чём шла \
        речь и какой главный итог. Без списков и заголовков, по-русски, по делу.";
    backend.chat(system, transcript_text)
}

/// Краткое резюме для разговора любой длины: длинный сначала сворачиваем в
/// выжимку (map-reduce), затем сжимаем до 2–4 предложений.
pub fn brief_summary_long(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    if transcript_text.len() <= SUMMARY_CHUNK_CHARS {
        return brief_summary(backend, transcript_text);
    }
    let digest = summarize_long(backend, transcript_text)?;
    let system = "Сожми эту выжимку до очень краткого резюме в 2–4 предложения: главная \
        суть и итог. Без списков и заголовков, по-русски.";
    backend.chat(system, &digest)
}

/// Аналитический разбор разговора (один проход).
pub fn analyze(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    let system = "Сделай аналитический разбор разговора в формате Markdown: позиции и \
        интересы сторон, ключевые аргументы, тон/настроение, принятые решения и \
        договорённости, открытые вопросы и разногласия, риски, рекомендуемые следующие \
        шаги. Заголовки (##) и списки. Опирайся только на сказанное, ничего не выдумывай. \
        По-русски.";
    backend.chat(system, transcript_text)
}

/// ИИ-анализ для разговора любой длины (map-reduce, как выжимка).
pub fn analyze_long(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    if transcript_text.len() <= SUMMARY_CHUNK_CHARS {
        return analyze(backend, transcript_text);
    }
    let chunks = split_chunks(transcript_text, SUMMARY_CHUNK_CHARS);
    let total = chunks.len();
    let mut partials = Vec::with_capacity(total);
    for (i, chunk) in chunks.iter().enumerate() {
        let system = "Проанализируй ОДНУ ЧАСТЬ длинного разговора: позиции сторон, \
            ключевые моменты, решения, разногласия и риски именно этого фрагмента. \
            По-русски, без вступлений.";
        let user = format!("Часть {}/{} разговора:\n\n{}", i + 1, total, chunk);
        partials.push(backend.chat(system, &user)?);
    }
    let combined = partials.join("\n\n---\n\n");
    if combined.len() > SUMMARY_CHUNK_CHARS {
        return analyze_long(backend, &combined);
    }
    let system = "Объедини анализы частей разговора в единый аналитический разбор \
        (Markdown: ## и списки): позиции сторон, ключевые аргументы, решения, открытые \
        вопросы, риски, следующие шаги. Без повторов и дублей. По-русски.";
    backend.chat(system, &combined)
}

/// Переписывает расшифровку в связный литературный текст (один проход).
pub fn to_literary_text(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    let system = "Перепиши расшифровку разговора в связный, гладкий литературный текст. \
        Сохрани ВСЕ обсуждаемые факты, детали, имена, числа и смысл; ничего не \
        выдумывай и не добавляй того, чего не было. Убери оговорки, повторы, \
        слова-паразиты и обрывы фраз. Оформи абзацами, по-русски. Без заголовков \
        и без вступлений вроде «в этом разговоре» — сразу текст.";
    backend.chat(system, transcript_text)
}

/// Литературный текст для разговора любой длины. Короткий — один проход;
/// длинный — переписываем каждую часть и СКЛЕИВАЕМ (не сворачиваем, в отличие
/// от выжимки), чтобы сохранить полноту изложения.
pub fn to_literary_long(backend: &dyn ChatBackend, transcript_text: &str) -> AppResult<String> {
    if transcript_text.len() <= SUMMARY_CHUNK_CHARS {
        return to_literary_text(backend, transcript_text);
    }
    let chunks = split_chunks(transcript_text, SUMMARY_CHUNK_CHARS);
    let total = chunks.len();
    let mut parts = Vec::with_capacity(total);
    for (i, chunk) in chunks.iter().enumerate() {
        let system = "Перепиши ЭТУ ЧАСТЬ разговора в связный литературный текст, \
            сохраняя все детали, факты и смысл именно этого фрагмента. Убери оговорки, \
            повторы и слова-паразиты. Без вступлений и выводов — только переложение \
            этой части. Оформи абзацами, по-русски.";
        let user = format!("Часть {}/{} разговора:\n\n{}", i + 1, total, chunk);
        parts.push(backend.chat(system, &user)?);
    }
    Ok(parts.join("\n\n"))
}

// ── Отчёты-пресеты (v0.8) ───────────────────────────────────────────────────
//
// Набор выстроен по задачам пользователя, без пересечений:
// - `summary`  «Итоги встречи»     — что обсудили и к чему пришли (главный отчёт);
// - `tasks`    «Задачи»            — только поручения: кто, что, к какому сроку;
// - `analysis` «Разбор разговора»  — как шёл разговор: позиции, тон, риски;
// - `literary` «Чистовой текст»    — весь разговор связным текстом без потерь;
// - `followup` «Письмо по итогам»  — готовое письмо участникам;
// - `agent`    «Инструкция для ИИ» — промпт для ИИ-агента (программирование и
//   т.п.): всё, что агенту нужно знать из разговора, чтобы сделать задачу.

/// Общие правила для всех отчётов.
const RULES: &str = "Пиши по-русски (если разговор на другом языке — на языке разговора). \
    Опирайся ТОЛЬКО на расшифровку: ничего не выдумывай, не додумывай имён, сроков и цифр; \
    если чего-то не прозвучало — так и пиши или пропускай. Расшифровка сделана автоматически \
    и может содержать ошибки распознавания — восстанавливай очевидный смысл, но не \
    сочиняй. Называй людей так, как они подписаны в расшифровке. Без вступлений \
    вроде «Вот итоги» — сразу результат в Markdown.";

/// Системный промпт отчёта вида `kind` или `None`, если вид неизвестен.
pub fn report_prompt(kind: &str) -> Option<String> {
    let body = match kind {
        "summary" => "Сделай итоги встречи. Структура (пустые разделы пропускай):\n\
            ## Коротко\n2–3 предложения: о чём был разговор и чем он закончился.\n\
            ## Главное\nКлючевые темы, факты и цифры — маркированный список, по пункту на мысль.\n\
            ## Решения\nЧто решили или согласовали.\n\
            ## Задачи\nСписок «- [ ] что сделать — кто — срок» (кто/срок — только если прозвучали).\n\
            ## Открытые вопросы\nЧто осталось нерешённым или требует уточнения.",
        "tasks" => "Извлеки из разговора все задачи, поручения и обещания что-то сделать.\n\
            ## Задачи\nТаблица Markdown с колонками: № | Задача | Ответственный | Срок | Откуда \
            (короткая цитата или контекст). Задача — с глагола, конкретно. Ответственный и срок — \
            только если прозвучали, иначе «—».\n\
            ## Договорённости\nСписок того, о чём договорились (без задач).\n\
            ## Нужно уточнить\nЗадачи без ответственного/срока и спорные моменты.\n\
            Если задач нет — так и напиши одной строкой.",
        "analysis" => "Сделай аналитический разбор разговора — не пересказ, а оценку.\n\
            ## Участники и их позиции\nЧего хотел каждый, какие интересы стояли за позицией.\n\
            ## Ход разговора\nКлючевые повороты, сильные аргументы, возражения и как на них ответили.\n\
            ## Тон и атмосфера\nНастрой, напряжение, доверие; на чём оно менялось.\n\
            ## Согласие и разногласия\n\
            ## Риски и сигналы\nЧто может пойти не так, недосказанности, противоречия.\n\
            ## Рекомендации\nКонкретные следующие шаги и что сделать иначе в следующий раз.",
        "literary" => "Перепиши расшифровку в связный, гладкий текст — как хорошо отредактированную \
            статью или конспект, который приятно читать. Сохрани ВСЕ факты, детали, имена, числа, \
            примеры и ход мысли; ничего не сокращай по смыслу. Убери оговорки, повторы, \
            слова-паразиты и обрывы фраз. Абзацы; подзаголовки (##) — только при смене темы. \
            Прямую речь передавай косвенно, указывая, кто говорит.",
        "followup" => "Напиши письмо участникам по итогам разговора — готовое к отправке.\n\
            Первая строка — «**Тема:** …». Далее: короткое приветствие; 1–2 предложения, о чём \
            говорили; «Договорились:» — список; «Следующие шаги:» — список «кто — что — срок»; \
            вежливое завершение. Деловой, дружелюбный тон, без канцелярита. Неизвестное \
            (имя адресата, дата) — в квадратных скобках, напр. [имя].",
        "agent" => "Составь из разговора максимально полезную инструкцию (промпт) для ИИ-агента, \
            который будет выполнять обсуждённую работу — например, писать код, готовить \
            документ или анализ. Агент не слышал разговор: передай ему всё нужное, убери \
            болтовню. Обращайся к агенту на «ты», в повелительном наклонении. Структура:\n\
            # Задача\nОдин абзац: что сделать и зачем (какую проблему решаем).\n\
            ## Контекст\nПроект/продукт, пользователи, текущее состояние, что уже есть и \
            что не работает — только факты из разговора.\n\
            ## Требования\nНумерованный список: конкретно, проверяемо, по одному требованию \
            в пункте. Приоритет — пометкой (обязательно / желательно).\n\
            ## Ограничения и договорённости\nТехнологии, стек, форматы, сроки, что \
            нельзя менять или делать, принятые решения и их причины.\n\
            ## Детали\nТочные имена, термины, числа, названия файлов/функций/экранов, \
            примеры и цитаты — дословно, как прозвучали.\n\
            ## Критерии готовности\nЧек-лист «- [ ] …»: по чему понять, что задача сделана.\n\
            ## Открытые вопросы\nЧто в разговоре не решено или неоднозначно — агент должен \
            уточнить это, а не додумывать.\n\
            Если в разговоре несколько независимых задач — сделай для каждой свой блок \
            «# Задача …». Если прямой рабочей задачи нет — сформулируй инструкцию для \
            продолжения темы разговора.",
        _ => return None,
    };
    Some(format!("{body}\n\n{RULES}"))
}

/// Человеческое название отчёта (для заголовков экспорта и ошибок).
pub fn report_title(kind: &str) -> &'static str {
    match kind {
        "summary" => "Итоги встречи",
        "tasks" => "Задачи",
        "analysis" => "Разбор разговора",
        "literary" => "Чистовой текст",
        "followup" => "Письмо по итогам",
        "agent" => "Инструкция для ИИ",
        "brief" => "Краткое резюме",
        _ => "Отчёт",
    }
}

/// Контекст встречи для модели: заголовок и участники (если заданы).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MeetingContext {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub participants: String,
    /// Имена говорящих, заданные пользователем: id ("me", "spk0"…) → имя.
    #[serde(default)]
    pub names: std::collections::HashMap<String, String>,
}

impl MeetingContext {
    fn header(&self) -> String {
        let mut h = String::new();
        if !self.title.trim().is_empty() {
            h.push_str(&format!("Встреча: {}\n", self.title.trim()));
        }
        if !self.participants.trim().is_empty() {
            h.push_str(&format!("Участники: {}\n", self.participants.trim()));
        }
        h
    }
}

/// Расшифровка в текст для модели: подряд идущие реплики одного говорящего
/// склеены, говорящие — по именам пользователя (иначе «Я», «Спикер 2»…).
pub fn transcript_to_named_text(
    transcript: &Transcript,
    names: &std::collections::HashMap<String, String>,
) -> String {
    let name_of = |id: &str| -> String {
        names
            .get(id)
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| speaker_label(id))
    };
    let mut out: Vec<String> = Vec::new();
    let mut last: Option<&str> = None;
    for s in &transcript.segments {
        let text = s.text.trim();
        if text.is_empty() {
            continue;
        }
        if last == Some(s.speaker.as_str()) {
            if let Some(l) = out.last_mut() {
                l.push(' ');
                l.push_str(text);
                continue;
            }
        }
        out.push(format!("{}: {}", name_of(&s.speaker), text));
        last = Some(s.speaker.as_str());
    }
    out.join("\n")
}

/// Строит отчёт вида `kind` по разговору любой длины. Короткий — один проход.
/// Длинный: `literary` переписывается по частям и склеивается (без потерь),
/// остальные — «map-reduce»: из каждой части выписываются подробные заметки
/// (факты, решения, задачи с исполнителями и сроками, цитаты), затем по
/// заметкам строится итоговый отчёт по тому же промпту.
pub fn generate_report(
    backend: &dyn ChatBackend,
    kind: &str,
    transcript_text: &str,
    ctx: &MeetingContext,
) -> AppResult<String> {
    let system = report_prompt(kind)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown report kind: {kind}")))?;
    let header = ctx.header();
    if transcript_text.len() <= SUMMARY_CHUNK_CHARS {
        let user = format!("{header}\nРасшифровка:\n{transcript_text}");
        return backend.chat(&system, &user);
    }

    let chunks = split_chunks(transcript_text, SUMMARY_CHUNK_CHARS);
    let total = chunks.len();
    if kind == "literary" {
        let mut parts = Vec::with_capacity(total);
        for (i, chunk) in chunks.iter().enumerate() {
            let user = format!(
                "{header}Это часть {}/{} длинного разговора. Перепиши ТОЛЬКО эту часть, \
                 без вступлений и выводов.\n\n{chunk}",
                i + 1,
                total
            );
            parts.push(backend.chat(&system, &user)?);
        }
        return Ok(parts.join("\n\n"));
    }

    let notes_system = format!(
        "Ты готовишь подробные рабочие заметки по ОДНОЙ ЧАСТИ длинного разговора, чтобы \
         потом по ним собрать итоговый отчёт. Выпиши: темы и ключевые факты/цифры, решения, \
         задачи и обещания (кто — что — срок), позиции и возражения участников, важные \
         цитаты, открытые вопросы. Кратко, списками, ничего важного не теряя.\n\n{RULES}"
    );
    let mut notes = Vec::with_capacity(total);
    for (i, chunk) in chunks.iter().enumerate() {
        let user = format!("{header}Часть {}/{} разговора:\n\n{chunk}", i + 1, total);
        notes.push(backend.chat(&notes_system, &user)?);
    }
    let mut combined = notes.join("\n\n---\n\n");
    // Заметок всё ещё слишком много — сжимаем их тем же способом.
    while combined.len() > SUMMARY_CHUNK_CHARS {
        let parts = split_chunks(&combined, SUMMARY_CHUNK_CHARS);
        if parts.len() <= 1 {
            break;
        }
        let mut next = Vec::with_capacity(parts.len());
        for p in &parts {
            next.push(backend.chat(&notes_system, p)?);
        }
        let joined = next.join("\n\n---\n\n");
        if joined.len() >= combined.len() {
            break;
        }
        combined = joined;
    }
    let user = format!(
        "{header}Ниже — заметки по частям одного длинного разговора (по порядку). \
         Собери по ним единый отчёт, без повторов.\n\n{combined}"
    );
    backend.chat(&system, &user)
}

/// Авто-заголовок с учётом контекста и имён говорящих.
pub fn suggest_metadata_ctx(
    backend: &dyn ChatBackend,
    transcript_text: &str,
) -> AppResult<MetadataSuggestion> {
    let system = "По расшифровке разговора верни СТРОГО JSON без пояснений с полями:\n\
        title — конкретный заголовок встречи в 3–7 слов, по сути разговора, без кавычек и \
        слов «встреча/разговор/обсуждение» в начале (пример: «Бюджет рекламы на III квартал»);\n\
        participants — имена участников через запятую, только если они прозвучали \
        (иначе пустая строка);\n\
        topic — тема одним коротким предложением.\n\
        Язык — язык разговора.";
    // Для заголовка хватает начала и конца длинного разговора.
    let text = if transcript_text.len() > SUMMARY_CHUNK_CHARS {
        let head: String = transcript_text.chars().take(SUMMARY_CHUNK_CHARS * 2 / 3).collect();
        let tail: String = {
            let v: Vec<char> = transcript_text.chars().collect();
            v[v.len().saturating_sub(SUMMARY_CHUNK_CHARS / 3)..].iter().collect()
        };
        format!("{head}\n…\n{tail}")
    } else {
        transcript_text.to_string()
    };
    let content = backend.chat(system, &text)?;
    let json = extract_json(&content);
    let mut suggestion: MetadataSuggestion = serde_json::from_str(json)?;
    suggestion.title = suggestion
        .title
        .trim()
        .trim_matches(|c| c == '"' || c == '«' || c == '»')
        .trim()
        .to_string();
    Ok(suggestion)
}

/// Ответ на вопрос пользователя по расшифровке.
pub fn answer_question(
    backend: &dyn ChatBackend,
    transcript_text: &str,
    question: &str,
) -> AppResult<String> {
    let system = "Отвечай на вопрос пользователя, опираясь на расшифровку разговора. \
        Если ответа в разговоре нет — так и скажи.";
    let user = format!("Расшифровка:\n{transcript_text}\n\nВопрос: {question}");
    backend.chat(system, &user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{merge_tracks, Segment};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Мок-бэкенд: отдаёт заданный ответ, запоминает последний (system,user)
    /// и считает число вызовов.
    struct MockChatBackend {
        response: String,
        last: Arc<Mutex<Option<(String, String)>>>,
        calls: Arc<Mutex<usize>>,
    }
    impl MockChatBackend {
        fn new(response: &str) -> Self {
            Self {
                response: response.into(),
                last: Arc::new(Mutex::new(None)),
                calls: Arc::new(Mutex::new(0)),
            }
        }
    }
    impl ChatBackend for MockChatBackend {
        fn chat(&self, system: &str, user: &str) -> AppResult<String> {
            *self.last.lock().unwrap() = Some((system.into(), user.into()));
            *self.calls.lock().unwrap() += 1;
            Ok(self.response.clone())
        }
    }

    #[test]
    fn summarize_long_short_input_is_single_pass() {
        let b = MockChatBackend::new("итог");
        let out = summarize_long(&b, "короткий разговор").unwrap();
        assert_eq!(out, "итог");
        assert_eq!(*b.calls.lock().unwrap(), 1);
    }

    #[test]
    fn summarize_long_chunks_then_reduces() {
        // Текст заметно длиннее порога → несколько частей + финальное объединение.
        let line = "Я: довольно длинная строка разговора для проверки разбиения\n";
        let big = line.repeat((SUMMARY_CHUNK_CHARS / line.len()) * 3 + 10);
        let b = MockChatBackend::new("короткий конспект");
        let out = summarize_long(&b, &big).unwrap();
        assert_eq!(out, "короткий конспект");
        // как минимум 3 части + 1 объединение = 4 вызова
        assert!(*b.calls.lock().unwrap() >= 4, "calls={}", *b.calls.lock().unwrap());
    }

    #[test]
    fn transcript_to_text_formats_speakers() {
        let t = merge_tracks(
            vec![Segment { start_secs: 0.0, end_secs: 1.0, text: "привет".into() }],
            vec![Segment { start_secs: 1.0, end_secs: 2.0, text: "хай".into() }],
        );
        assert_eq!(transcript_to_text(&t), "Я: привет\nСобеседник: хай");
    }

    #[test]
    fn suggest_metadata_parses_plain_json() {
        let b = MockChatBackend::new(
            r#"{"title":"Созвон","participants":"Иван, Пётр","topic":"Планы"}"#,
        );
        let s = suggest_metadata(&b, "Я: ...").unwrap();
        assert_eq!(s.title, "Созвон");
        assert_eq!(s.participants, "Иван, Пётр");
        assert_eq!(s.topic, "Планы");
    }

    #[test]
    fn suggest_metadata_parses_json_in_fenced_block_with_extra_text() {
        let b = MockChatBackend::new(
            "Вот результат:\n```json\n{\"title\":\"X\",\"participants\":\"A\",\"topic\":\"Y\"}\n```\nГотово.",
        );
        let s = suggest_metadata(&b, "t").unwrap();
        assert_eq!(s.title, "X");
        assert_eq!(s.topic, "Y");
    }

    #[test]
    fn suggest_metadata_tolerates_array_field() {
        // Модель вернула participants массивом — раньше это роняло десериализацию.
        let b = MockChatBackend::new(
            r#"{"title":"Созвон","participants":["Иван","Пётр"],"topic":"Планы"}"#,
        );
        let s = suggest_metadata(&b, "t").unwrap();
        assert_eq!(s.title, "Созвон");
        assert_eq!(s.participants, "Иван, Пётр");
        assert_eq!(s.topic, "Планы");
    }

    #[test]
    fn summarize_and_answer_pass_through_backend() {
        let b = MockChatBackend::new("краткая выжимка");
        assert_eq!(summarize(&b, "текст").unwrap(), "краткая выжимка");
        let last = b.last.lock().unwrap().clone().unwrap();
        assert!(last.1.contains("текст")); // user content carries transcript

        let b2 = MockChatBackend::new("ответ");
        assert_eq!(answer_question(&b2, "разговор", "вопрос?").unwrap(), "ответ");
        let last2 = b2.last.lock().unwrap().clone().unwrap();
        assert!(last2.1.contains("разговор") && last2.1.contains("вопрос?"));
    }

    #[test]
    fn brief_and_analyze_short_are_single_pass() {
        let b = MockChatBackend::new("кратко");
        assert_eq!(brief_summary_long(&b, "короткий разговор").unwrap(), "кратко");
        assert_eq!(*b.calls.lock().unwrap(), 1);

        let a = MockChatBackend::new("анализ");
        assert_eq!(analyze_long(&a, "короткий разговор").unwrap(), "анализ");
        assert_eq!(*a.calls.lock().unwrap(), 1);
    }

    #[test]
    fn literary_short_is_single_pass() {
        let b = MockChatBackend::new("связный текст");
        let out = to_literary_long(&b, "Я: привет\nСобеседник: здравствуй").unwrap();
        assert_eq!(out, "связный текст");
        assert_eq!(*b.calls.lock().unwrap(), 1);
    }

    #[test]
    fn literary_long_rewrites_each_chunk_and_joins() {
        let line = "Я: довольно длинная строка разговора для проверки разбиения\n";
        let big = line.repeat((SUMMARY_CHUNK_CHARS / line.len()) * 2 + 10);
        let b = MockChatBackend::new("часть");
        let out = to_literary_long(&b, &big).unwrap();
        // Склейка (не сворачивание): несколько частей через пустую строку.
        assert!(out.contains("часть\n\nчасть"), "joined output: {out:?}");
        assert!(*b.calls.lock().unwrap() >= 2);
    }

    /// Поднимает локальный HTTP-сервер, отвечающий OpenAI-подобным JSON.
    /// Читает HTTP-запрос целиком: заголовки и тело по Content-Length (тело
    /// может прийти отдельным пакетом).
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut data: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data).to_string();
            if let Some(h) = text.find("\r\n\r\n") {
                let len = text[..h]
                    .lines()
                    .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if data.len() >= h + 4 + len {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&data).to_string()
    }

    fn spawn_mock_server(content: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = format!(
            r#"{{"choices":[{{"message":{{"content":{}}}}}]}}"#,
            serde_json::to_string(content).unwrap()
        );
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Весь запрос до ответа: иначе непрочитанное тело при закрытии
                // сокета даёт RST, и клиент (на macOS) падает с os error 22.
                let _ = read_request(&mut stream);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn every_preset_has_prompt_and_title() {
        for k in ["summary", "tasks", "analysis", "literary", "followup", "agent"] {
            let p = report_prompt(k).unwrap();
            assert!(p.contains("ничего не выдумывай"), "{k}");
            assert_ne!(report_title(k), "Отчёт");
        }
        assert!(report_prompt("brief").is_none());
        assert!(report_prompt("../x").is_none());
    }

    #[test]
    fn generate_report_short_single_pass_with_context() {
        let b = MockChatBackend::new("## Коротко\nок");
        let ctx = MeetingContext {
            title: "Бюджет".into(),
            participants: "Иван".into(),
            names: Default::default(),
        };
        let out = generate_report(&b, "summary", "Иван: привет", &ctx).unwrap();
        assert_eq!(out, "## Коротко\nок");
        assert_eq!(*b.calls.lock().unwrap(), 1);
        let (sys, user) = b.last.lock().unwrap().clone().unwrap();
        assert!(sys.contains("## Решения"));
        assert!(user.contains("Встреча: Бюджет") && user.contains("Иван: привет"));
        assert!(generate_report(&b, "nope", "x", &ctx).is_err());
    }

    #[test]
    fn generate_report_long_uses_notes_then_reduce() {
        let line = "Я: довольно длинная строка разговора для проверки разбиения\n";
        let big = line.repeat((SUMMARY_CHUNK_CHARS / line.len()) * 2 + 10);
        let b = MockChatBackend::new("заметки");
        let out = generate_report(&b, "tasks", &big, &MeetingContext::default()).unwrap();
        assert_eq!(out, "заметки");
        assert!(*b.calls.lock().unwrap() >= 3);
        let (sys, user) = b.last.lock().unwrap().clone().unwrap();
        assert!(sys.contains("Ответственный"));
        assert!(user.contains("заметки по частям"));

        let lit = MockChatBackend::new("часть");
        let out = generate_report(&lit, "literary", &big, &MeetingContext::default()).unwrap();
        assert!(out.contains("часть\n\nчасть"));
    }

    #[test]
    fn named_text_uses_labels_and_merges_runs() {
        let t = merge_tracks(
            vec![
                Segment { start_secs: 0.0, end_secs: 1.0, text: "привет".into() },
                Segment { start_secs: 1.0, end_secs: 2.0, text: "как ты".into() },
            ],
            vec![Segment { start_secs: 3.0, end_secs: 4.0, text: "норм".into() }],
        );
        let mut names = std::collections::HashMap::new();
        names.insert("them".to_string(), "Олег".to_string());
        assert_eq!(transcript_to_named_text(&t, &names), "Я: привет как ты\nОлег: норм");
    }

    #[test]
    fn suggest_metadata_ctx_strips_quotes() {
        let b = MockChatBackend::new(r#"{"title":"«Бюджет на Q3»","participants":"","topic":"t"}"#);
        let s = suggest_metadata_ctx(&b, "x").unwrap();
        assert_eq!(s.title, "Бюджет на Q3");
    }

    #[test]
    fn pick_model_prefers_configured_then_closest() {
        let av = vec!["qwen3.6-27b-q4".to_string(), "llama-3.1-8b".to_string()];
        assert_eq!(pick_model("llama-3.1-8b", &av).as_deref(), Some("llama-3.1-8b"));
        // Выкатили новую версию — старое имя исчезло, берём ближайшее.
        assert_eq!(pick_model("qwen3.5-27b-q4", &av).as_deref(), Some("qwen3.6-27b-q4"));
        assert_eq!(pick_model("", &av).as_deref(), Some("qwen3.6-27b-q4"));
        assert_eq!(pick_model("auto", &av).as_deref(), Some("qwen3.6-27b-q4"));
        assert_eq!(pick_model("zzz", &av).as_deref(), Some("qwen3.6-27b-q4"));
        assert_eq!(pick_model("x", &[]), None);
    }

    #[test]
    fn strip_think_removes_reasoning() {
        assert_eq!(strip_think("<think>hmm\nплан</think>\n\n## Итоги"), "## Итоги");
        assert_eq!(strip_think("просто ответ"), "просто ответ");
    }

    /// Мок-сервер на несколько запросов: `/models` отдаёт список, а
    /// `/chat/completions` отвечает 404 для модели `old`, иначе — текстом.
    fn spawn_model_server(models: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming().take(6) {
                let Ok(mut stream) = stream else { continue };
                let req = read_request(&mut stream);
                seen2.lock().unwrap().push(req.lines().next().unwrap_or("").to_string());
                let (status, body) = if req.starts_with("GET /models") {
                    ("200 OK", models.to_string())
                } else if req.contains("\"model\":\"old\"") {
                    ("404 Not Found", r#"{"error":{"message":"model 'old' not found"}}"#.to_string())
                } else {
                    let model = if req.contains("\"model\":\"new-2\"") { "new-2" } else { "?" };
                    ("200 OK", format!(r#"{{"choices":[{{"message":{{"content":"ответ от {model}"}}}}]}}"#))
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://127.0.0.1:{port}"), seen)
    }

    #[test]
    fn backend_follows_updated_server_model() {
        let (base, _seen) = spawn_model_server(r#"{"data":[{"id":"new-2"}]}"#);
        let b = HttpChatBackend::new(AiConfig { base_url: base, api_key: "k".into(), model: "old".into() });
        assert_eq!(b.chat("s", "u").unwrap(), "ответ от new-2");
        assert_eq!(b.used_model().as_deref(), Some("new-2"));
    }

    #[test]
    fn backend_auto_model_and_check() {
        let (base, _) = spawn_model_server(r#"{"models":[{"name":"new-2"}]}"#);
        let cfg = AiConfig { base_url: base, api_key: "k".into(), model: "".into() };
        let b = HttpChatBackend::new(cfg);
        assert_eq!(b.chat("s", "u").unwrap(), "ответ от new-2");

        let (base, _) = spawn_model_server(r#"{"data":[{"id":"new-2"}]}"#);
        let c = check(&AiConfig { base_url: base, api_key: "k".into(), model: "old".into() });
        assert!(c.ok && c.changed);
        assert_eq!(c.model, "new-2");
        assert_eq!(c.models, vec!["new-2".to_string()]);

        let bad = check(&AiConfig { base_url: "http://127.0.0.1:1".into(), api_key: "k".into(), model: "m".into() });
        assert!(!bad.ok && bad.error.is_some());
    }

    #[test]
    fn http_backend_sends_request_and_parses_response() {
        let base = spawn_mock_server("Привет из мок-сервера");
        let backend = HttpChatBackend::new(AiConfig {
            base_url: base,
            api_key: "k".into(),
            model: "m".into(),
        });
        let out = backend.chat("sys", "user").unwrap();
        assert_eq!(out, "Привет из мок-сервера");
    }
}
