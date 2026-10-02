import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { TranscriptView } from "../components/TranscriptView";

const transcript = {
  segments: [
    { speaker: "spk0", start_secs: 0, end_secs: 1, text: "Привет" },
    { speaker: "spk0", start_secs: 1, end_secs: 2, text: "Как дела" },
    { speaker: "spk1", start_secs: 2, end_secs: 3, text: "Хорошо" },
  ],
};

describe("TranscriptView speaker menu", () => {
  it("changes the speaker of one phrase without seeking", async () => {
    const onReassign = vi.fn();
    const onSeek = vi.fn();
    render(
      <TranscriptView
        transcript={transcript}
        activeIndex={-1}
        labels={{ spk1: "Анна" }}
        onSeek={onSeek}
        speakerOptions={["spk0", "spk1"]}
        onReassign={onReassign}
      />,
    );
    await userEvent.click(screen.getAllByTitle("Сменить говорящего")[0]);
    expect(screen.getByRole("menu")).toBeTruthy();
    await userEvent.click(screen.getByRole("menuitemradio", { name: /Анна/ }));
    expect(onReassign).toHaveBeenCalledWith(0, "spk1", false);
    expect(onSeek).not.toHaveBeenCalled();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("offers a new voice and the following run", async () => {
    const onReassign = vi.fn();
    render(
      <TranscriptView
        transcript={transcript}
        activeIndex={-1}
        labels={{}}
        onSeek={() => {}}
        speakerOptions={["spk0", "spk1"]}
        onReassign={onReassign}
      />,
    );
    await userEvent.click(screen.getAllByTitle("Сменить говорящего")[0]);
    await userEvent.click(screen.getByLabelText(/и следующие подряд \(1\)/));
    await userEvent.click(screen.getByRole("menuitem", { name: /Новый голос/ }));
    expect(onReassign).toHaveBeenCalledWith(0, null, true);
  });
});

describe("TranscriptView inline edit and gaps", () => {
  it("edits one phrase in place and saves with Ctrl+Enter", async () => {
    const onSaveText = vi.fn();
    const onSeek = vi.fn();
    render(
      <TranscriptView
        transcript={transcript}
        activeIndex={-1}
        labels={{}}
        onSeek={onSeek}
        onSaveText={onSaveText}
        rawText={(i) => transcript.segments[i].text}
      />,
    );
    await userEvent.click(screen.getAllByLabelText("Исправить текст реплики")[1]);
    const box = screen.getByPlaceholderText("Что было сказано…");
    await userEvent.clear(box);
    await userEvent.type(box, "Как ваши дела{Control>}{Enter}{/Control}");
    expect(onSaveText).toHaveBeenCalledWith(1, "Как ваши дела", undefined);
    expect(onSeek).not.toHaveBeenCalled();
  });

  it("shows gaps with listen / recognize / add actions", async () => {
    const onPlayRange = vi.fn();
    const onRecognizeGap = vi.fn(async () => {});
    const onAddManual = vi.fn(async () => 1);
    const gap = { after: 0, start: 1, end: 6 };
    render(
      <TranscriptView
        transcript={transcript}
        activeIndex={-1}
        labels={{}}
        onSeek={() => {}}
        gaps={[gap]}
        onPlayRange={onPlayRange}
        onRecognizeGap={onRecognizeGap}
        onAddManual={onAddManual}
        onSaveText={() => {}}
      />,
    );
    expect(screen.getByText(/Пропуск 0:01–0:06/)).toBeTruthy();
    await userEvent.click(screen.getByText("▶ Послушать"));
    expect(onPlayRange).toHaveBeenCalledWith(1, 6);
    await userEvent.click(screen.getByText("↻ Распознать"));
    expect(onRecognizeGap).toHaveBeenCalledWith(gap);
    await userEvent.click(screen.getByText("＋ Дописать"));
    expect(onAddManual).toHaveBeenCalledWith(gap);
    expect(await screen.findByPlaceholderText("Что было сказано…")).toBeTruthy();
  });

  it("adds an empty bubble after a phrase and picks who speaks", async () => {
    const onSaveText = vi.fn();
    const withNew = {
      segments: [
        transcript.segments[0],
        { speaker: "spk0", start_secs: 1, end_secs: 2, text: "", origin: "user" as const },
        ...transcript.segments.slice(1),
      ],
    };
    const onAddAfter = vi.fn(() => 1);
    render(
      <TranscriptView
        transcript={withNew}
        activeIndex={-1}
        labels={{ spk1: "Анна" }}
        onSeek={() => {}}
        speakerOptions={["spk0", "spk1"]}
        onSaveText={onSaveText}
        onAddAfter={onAddAfter}
      />,
    );
    await userEvent.click(screen.getAllByLabelText("Добавить реплику после этой")[0]);
    expect(onAddAfter).toHaveBeenCalledWith(0);
    expect(screen.getByRole("radiogroup", { name: "Кто говорит" })).toBeTruthy();
    await userEvent.click(screen.getByRole("radio", { name: /Анна/ }));
    await userEvent.type(screen.getByPlaceholderText("Впишите, что слышно в записи…"), "Я тоже за");
    await userEvent.click(screen.getByText("Сохранить"));
    expect(onSaveText).toHaveBeenCalledWith(1, "Я тоже за", "spk1");
  });

  it("marks user phrases and explains why one was missed", () => {
    render(
      <TranscriptView
        transcript={{
          segments: [
            { speaker: "spk0", start_secs: 0, end_secs: 1, text: "Дописал", origin: "user" },
            { speaker: "spk1", start_secs: 2, end_secs: 3, text: "Исправил", origin: "edited" },
          ],
        }}
        activeIndex={-1}
        labels={{}}
        onSeek={() => {}}
        whyMissed={(i) => (i === 0 ? "говорили одновременно" : undefined)}
      />,
    );
    expect(screen.getByText("добавлено вами")).toBeTruthy();
    expect(screen.getByText("исправлено")).toBeTruthy();
    expect(screen.getByText("Почему не распозналось: говорили одновременно")).toBeTruthy();
  });
});
