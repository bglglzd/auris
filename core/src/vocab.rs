//! Словарь терминов: имена, продукты, команды, профессиональные слова —
//! часто на другом языке, чем речь («созвонимся в Zoom», «залей в GitHub»).
//!
//! Работает локально и двумя способами:
//! - подсказка Whisper (`initial_prompt`) — модель охотнее пишет термин верно;
//! - исправление после распознавания: слово, похожее на термин (в том числе
//!   записанное кириллицей: «джира» → Jira, «кубернетес» → Kubernetes),
//!   заменяется написанием из словаря.
//!
//! Сравнение строгое (почти совпадение после транслитерации), короткие слова
//! (< 4 букв) не трогаются — чтобы не портить обычную речь.

/// Встроенный словарь: сервисы и продукты, у которых каноническое написание —
/// латиницей. Только достаточно своеобразные имена (без обычных слов).
pub const BUILTIN: &[&str] = &[
    "Zoom", "Google Meet", "Microsoft Teams", "Skype", "Telegram", "WhatsApp", "Slack", "Discord",
    "Jira", "Confluence", "GitHub", "GitLab", "Bitbucket", "Notion", "Figma", "Miro", "Trello",
    "Asana", "YouTrack", "Kubernetes", "Docker", "Terraform", "Ansible", "Jenkins", "Grafana",
    "Prometheus", "Kafka", "RabbitMQ", "PostgreSQL", "MySQL", "MongoDB", "ClickHouse",
    "Elasticsearch", "Python", "JavaScript", "TypeScript", "Kotlin", "Swift", "React", "Angular",
    "Node.js", "Linux", "Windows", "macOS", "Android", "iPhone", "iPad", "Excel", "PowerPoint",
    "Outlook", "OneDrive", "SharePoint", "Google Docs", "Google Sheets", "Google Drive", "Dropbox",
    "Salesforce", "HubSpot", "Bitrix24", "amoCRM", "Tableau", "Power BI", "ChatGPT", "OpenAI",
    "Claude", "Copilot", "Yandex", "Wildberries", "Avito", "Tinkoff", "Sber",
];

/// Термины из текста настроек: по строке или через запятую/точку с запятой.
pub fn parse_terms(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in text.split(['\n', ',', ';']) {
        let t = t.trim();
        if t.chars().filter(|c| c.is_alphanumeric()).count() >= 2 && !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    out
}

/// Словарь: термины пользователя (важнее) + встроенные.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    pub user: Vec<String>,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
struct Entry {
    term: String,
    /// Термин на кириллице: только точное совпадение (иначе «Петрова» стало
    /// бы «Петров» — падежи не трогаем).
    exact: bool,
    /// Ключ сравнения: латиница в нижнем регистре без пробелов/знаков.
    key: String,
    words: usize,
}

impl Vocabulary {
    pub fn new(user_text: &str, builtin: bool) -> Self {
        let user = parse_terms(user_text);
        let mut entries: Vec<Entry> = Vec::new();
        let all = user.iter().map(String::as_str).chain(if builtin { BUILTIN } else { &[] }.iter().copied());
        for term in all {
            let key = key_of(term);
            if key.chars().count() < 4 || entries.iter().any(|e| e.key == key) {
                continue;
            }
            let exact = term.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
            entries.push(Entry { term: term.to_string(), exact, key, words: term.split_whitespace().count().max(1) });
        }
        Self { user, entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Подсказка для Whisper: термины пользователя (до ~400 символов).
    pub fn prompt(&self) -> Option<String> {
        let mut s = String::new();
        for t in &self.user {
            if s.len() + t.len() > 400 {
                break;
            }
            if !s.is_empty() {
                s.push_str(", ");
            }
            s.push_str(t);
        }
        (!s.is_empty()).then(|| format!("{s}."))
    }

    /// Исправляет написание терминов в тексте реплики.
    pub fn correct(&self, text: &str) -> String {
        if self.entries.is_empty() {
            return text.to_string();
        }
        let tokens: Vec<&str> = text.split(' ').collect();
        let mut out: Vec<String> = Vec::with_capacity(tokens.len());
        let mut i = 0;
        'outer: while i < tokens.len() {
            // Сначала многословные термины (длиннее — раньше).
            for n in (1..=3).rev() {
                if i + n > tokens.len() {
                    continue;
                }
                let group = &tokens[i..i + n];
                let (lead, _) = split_punct(group[0]);
                let (_, trail) = split_punct(group[n - 1]);
                let core: String = group.join(" ");
                let key = key_of(&core);
                if key.chars().count() < 4 {
                    continue;
                }
                if let Some(e) = self.best(&key, n) {
                    out.push(format!("{lead}{}{trail}", e.term));
                    i += n;
                    continue 'outer;
                }
            }
            out.push(tokens[i].to_string());
            i += 1;
        }
        out.join(" ")
    }

    fn best(&self, key: &str, words: usize) -> Option<&Entry> {
        let len = key.chars().count();
        self.entries
            .iter()
            .filter(|e| e.words == words)
            .filter(|e| e.key.chars().next() == key.chars().next() || first_sound_eq(&e.key, key))
            .filter_map(|e| {
                let d = levenshtein(&e.key, key);
                let max_len = len.max(e.key.chars().count());
                let allowed = if e.exact || max_len <= 4 { 0 } else if max_len <= 7 { 1 } else { 2 };
                (d <= allowed).then_some((d, e))
            })
            .min_by_key(|(d, _)| *d)
            .map(|(_, e)| e)
    }
}

/// «к»/«c», «ф»/«ph» в начале слова звучат одинаково: Kafka ~ «кафка», Confluence ~ «конфлюенс».
fn first_sound_eq(a: &str, b: &str) -> bool {
    let f = |s: &str| match s.chars().next() {
        Some('c') | Some('k') | Some('q') => 'k',
        Some(c) => c,
        None => ' ',
    };
    f(a) == f(b)
}

/// Отделяет ведущие/хвостовые знаки препинания: «(Jira),» → ("(", ")," ).
fn split_punct(w: &str) -> (&str, &str) {
    let start = w.find(|c: char| c.is_alphanumeric()).unwrap_or(w.len());
    let end = w.rfind(|c: char| c.is_alphanumeric()).map(|i| i + w[i..].chars().next().unwrap().len_utf8()).unwrap_or(start);
    (&w[..start], &w[end.max(start)..])
}

/// Ключ сравнения: транслитерация кириллицы, нижний регистр, только буквы/цифры.
pub fn key_of(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let chars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // «дж» → j (джира → jira), «кс» → x (эксель → exel ~ excel).
        if c == 'д' && chars.get(i + 1) == Some(&'ж') {
            out.push('j');
            i += 2;
            continue;
        }
        let t: &str = match c {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' => "e", 'ё' => "yo",
            'ж' => "zh", 'з' => "z", 'и' => "i", 'й' => "y", 'к' => "k", 'л' => "l", 'м' => "m",
            'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u",
            'ф' => "f", 'х' => "h", 'ц' => "ts", 'ч' => "ch", 'ш' => "sh", 'щ' => "sch", 'ъ' => "",
            'ы' => "y", 'ь' => "", 'э' => "e", 'ю' => "yu", 'я' => "ya", 'є' => "ye", 'і' => "i",
            'ї' => "yi", 'ґ' => "g",
            c if c.is_ascii_alphanumeric() => {
                out.push(c);
                i += 1;
                continue;
            }
            _ => "",
        };
        out.push_str(t);
        i += 1;
    }
    // Двойные буквы и немые окончания в произношении не слышны.
    let mut dedup = String::with_capacity(out.len());
    for c in out.chars() {
        if !dedup.ends_with(c) {
            dedup.push(c);
        }
    }
    dedup.replace("ck", "k").replace("ph", "f").replace("th", "t").replace("ee", "i").replace("oo", "u")
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terms() {
        assert_eq!(parse_terms("Jira, деплой\nKubernetes; jira\n x"), ["Jira", "деплой", "Kubernetes"]);
    }

    #[test]
    fn fixes_cyrillic_spellings_of_products() {
        let v = Vocabulary::new("", true);
        assert_eq!(v.correct("Заведи задачу в джира, ок?"), "Заведи задачу в Jira, ок?");
        assert_eq!(v.correct("кубернетес упал"), "Kubernetes упал");
        assert_eq!(v.correct("созвонимся в зум"), "созвонимся в зум"); // короткое — не трогаем
        assert_eq!(v.correct("скинь в слак"), "скинь в Slack");
        assert_eq!(v.correct("залей на гитхаб."), "залей на GitHub.");
        assert_eq!(v.correct("обсудим в google meet"), "обсудим в Google Meet");
    }

    #[test]
    fn leaves_ordinary_words_alone() {
        let v = Vocabulary::new("", true);
        for s in [
            "Давайте обсудим план на следующую неделю",
            "мы подготовили презентацию и расчёты к пятнице",
            "сначала проверим документы, потом договор",
            "окно, слово, дело, поиск, работа, команда",
        ] {
            assert_eq!(v.correct(s), s);
        }
    }

    #[test]
    fn user_terms_and_prompt() {
        let v = Vocabulary::new("Memiro\nПетров Алексей\nOKR-board", false);
        assert_eq!(v.correct("открой мемиро"), "открой Memiro");
        assert_eq!(v.correct("спроси у петрова алексея"), "спроси у петрова алексея"); // падеж — не наша задача
        assert_eq!(v.prompt().as_deref(), Some("Memiro, Петров Алексей, OKR-board."));
        assert!(Vocabulary::new("", false).prompt().is_none());
    }
}
