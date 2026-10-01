//! Нецензурная лексика: политика «скрывать» заменяет мат пометкой
//! `[нецензурно]`. Расшифровка хранится дословно — замена делается при показе,
//! экспорте и отправке текста ИИ, поэтому политику можно сменить в любой
//! момент без потери данных.
//!
//! Правила — по корням с допустимыми приставками (чтобы не задеть «страхуй»,
//! «колебать», «вебинар», «мудрый» и т.п.). Та же логика — во фронтенде
//! (`src/profanity.ts`); при изменении правил менять оба файла и общие тесты.

/// Пометка вместо скрытого слова.
pub const MARK: &str = "[нецензурно]";

const HUI_PREFIXES: &[&str] = &[
    "", "по", "на", "за", "от", "до", "вы", "из", "ис", "об", "о", "при", "про", "раз", "рас", "под",
    "пере", "недо", "ни", "не", "а", "охуе",
];
const HUI_ROOTS: &[&str] = &["хуй", "хуе", "хуя", "хуи", "хую", "хул"];

const EB_PREFIXES: &[&str] = &[
    "", "вы", "за", "на", "по", "от", "отъ", "до", "у", "при", "про", "пере", "недо", "долбо", "съ",
    "въ", "взъ", "объ", "подъ", "изъ", "разъ", "раз",
];

/// Матерное ли слово (буквы в любом регистре, «ё» = «е»).
pub fn is_obscene(word: &str) -> bool {
    let w: String = word.to_lowercase().replace('ё', "е");
    let w = w.as_str();
    if w.is_empty() {
        return false;
    }
    if ["пизд", "залуп", "гандон", "гондон", "бляд", "блят", "мандавош", "motherfuck", "fuck"]
        .iter()
        .any(|r| w.contains(r))
    {
        return true;
    }
    if ["бля", "блять", "сука", "суки", "суку", "сукой", "сучара", "cunt", "asshole", "bullshit"].contains(&w) {
        return true;
    }
    if ["пидор", "пидар", "пидр", "мудак", "мудил", "мудач", "мудозвон", "шлюх", "сукин", "shit", "bitch"]
        .iter()
        .any(|r| w.starts_with(r))
    {
        return true;
    }
    // «хуй»: корень в начале слова или после приставки («похуй», «охуеть»),
    // но не внутри обычных слов («страхуй», «психуй»).
    for p in HUI_PREFIXES {
        if let Some(rest) = w.strip_prefix(p) {
            if HUI_ROOTS.iter().any(|r| rest.starts_with(r)) {
                // «хул…» — только «хуля/хули» (а не «хулиган», «хулить»).
                if rest.starts_with("хул") && !["хуля", "хули", "хуле"].contains(&rest) {
                    continue;
                }
                return true;
            }
        }
    }
    // «еб…»: в начале слова или после приставки («заебал», «выебон»), но
    // не «колебать», «вебинар», «хлебать», «учебник», «себе».
    for p in EB_PREFIXES {
        if let Some(rest) = w.strip_prefix(p) {
            if rest.starts_with("еб") && !rest.starts_with("ебонит") {
                return true;
            }
        }
    }
    false
}

/// Заменяет нецензурные слова пометкой [`MARK`]; остальной текст — как есть.
pub fn censor(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            if is_obscene(word) {
                out.push_str(MARK);
            } else {
                out.push_str(word);
            }
            word.clear();
        }
    };
    for c in text.chars() {
        if c.is_alphabetic() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Общие примеры — те же, что в `src/test/profanity.test.ts`.
    const OBSCENE: &[&str] = &[
        "хуй", "Похуй", "нахуя", "охуенно", "нихуя", "пизда", "распиздяй", "ебать", "Заебал", "выебон",
        "проебали", "долбоеб", "уебище", "съебался", "ёбаный", "блядь", "бля", "мудак", "пидор", "сука",
        "залупа", "fuck", "Fucking", "shit", "хуле", "хули",
    ];
    const CLEAN: &[&str] = &[
        "страхуй", "психуй", "хулиган", "колебать", "вебинар", "хлебать", "учебник", "себе", "небо",
        "ребята", "требовать", "погребать", "мудрый", "победа", "бляха", "сукно", "Скупой", "хобби",
        "ебонит", "дебаты",
    ];

    #[test]
    fn detects_obscene_words() {
        for w in OBSCENE {
            assert!(is_obscene(w), "должно скрываться: {w}");
        }
    }

    #[test]
    fn keeps_ordinary_words() {
        for w in CLEAN {
            assert!(!is_obscene(w), "не должно скрываться: {w}");
        }
    }

    #[test]
    fn censors_inside_text() {
        assert_eq!(censor("Ну это, блядь, полный пиздец!"), "Ну это, [нецензурно], полный [нецензурно]!");
        assert_eq!(censor("Застрахуйте машину до пятницы."), "Застрахуйте машину до пятницы.");
        assert_eq!(censor(""), "");
    }
}
