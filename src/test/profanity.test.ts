import { describe, it, expect } from "vitest";
import { applyPolicy, censor, isObscene } from "../profanity";

// Те же примеры, что в core/src/profanity.rs.
const OBSCENE = [
  "хуй", "Похуй", "нахуя", "охуенно", "нихуя", "пизда", "распиздяй", "ебать", "Заебал", "выебон",
  "проебали", "долбоеб", "уебище", "съебался", "ёбаный", "блядь", "бля", "мудак", "пидор", "сука",
  "залупа", "fuck", "Fucking", "shit", "хуле", "хули",
];
const CLEAN = [
  "страхуй", "психуй", "хулиган", "колебать", "вебинар", "хлебать", "учебник", "себе", "небо",
  "ребята", "требовать", "погребать", "мудрый", "победа", "бляха", "сукно", "Скупой", "хобби",
  "ебонит", "дебаты",
];

describe("profanity", () => {
  it("hides obscene words", () => {
    for (const w of OBSCENE) expect(isObscene(w), w).toBe(true);
  });
  it("keeps ordinary words", () => {
    for (const w of CLEAN) expect(isObscene(w), w).toBe(false);
  });
  it("censors in text and only under the censor policy", () => {
    expect(censor("Ну это, блядь, полный пиздец!")).toBe("Ну это, [нецензурно], полный [нецензурно]!");
    expect(censor("Застрахуйте машину до пятницы.")).toBe("Застрахуйте машину до пятницы.");
    const t = { segments: [{ speaker: "me", start_secs: 0, end_secs: 1, text: "ну блядь" }] };
    expect(applyPolicy(t, "censor").segments[0].text).toBe("ну [нецензурно]");
    expect(applyPolicy(t, "verbatim")).toBe(t);
    expect(t.segments[0].text).toBe("ну блядь");
  });
});
