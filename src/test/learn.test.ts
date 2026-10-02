import { describe, expect, it } from "vitest";
import { learnedLabel, learnFromEdit, mergeLearned } from "../learn";

describe("learnFromEdit", () => {
  it("learns misheard terms as replacement rules", () => {
    expect(learnFromEdit("Заведём задачу в жиру, потом в кубер.", "Заведём задачу в Jira, потом в Kubernetes.")).toEqual([
      "жиру => Jira",
      "кубер => Kubernetes",
    ]);
    // Имя посреди фразы.
    expect(learnFromEdit("Спроси у петрова завтра", "Спроси у Петрова завтра")).toEqual(["петрова => Петрова"]);
  });

  it("does not turn ordinary word fixes into global rules", () => {
    expect(learnFromEdit("Золотая цепь над дубитом", "Золотая цепь на дубе том")).toEqual([]);
    // Заглавная в начале предложения — не термин.
    expect(learnFromEdit("ну да. потом", "ну да. Потом")).toEqual([]);
    expect(learnFromEdit("одно и то же", "одно и то же")).toEqual([]);
  });

  it("learns terms from a phrase the user added", () => {
    expect(learnFromEdit("", "Скинь ссылку в Slack и в чат с Ириной.")).toEqual(["Slack", "Ириной"]);
  });
});

describe("mergeLearned", () => {
  it("dedupes, replaces rules for the same source and keeps the newest", () => {
    const a = mergeLearned("", ["жира => Jira", "Slack"]);
    expect(a).toBe("жира => Jira\nSlack");
    expect(mergeLearned(a, ["Жира => JIRA", "slack"])).toBe("Жира => JIRA\nslack");
    expect(learnedLabel(["жира => Jira", "Slack"])).toBe("жира → Jira, Slack");
  });
});
