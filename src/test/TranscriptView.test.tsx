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
