import { describe, it, expect } from "vitest";
import { formatClock } from "../util";

describe("formatClock", () => {
  it('formats 0 as "0:00"', () => {
    expect(formatClock(0)).toBe("0:00");
  });

  it('formats 65 as "1:05"', () => {
    expect(formatClock(65)).toBe("1:05");
  });

  it('formats negative numbers as "0:00"', () => {
    expect(formatClock(-5)).toBe("0:00");
  });
});

import { balancedColumns } from "../util";

describe("balancedColumns", () => {
  it("fills rows evenly for 6 chips", () => {
    // 6 плашек: 6 в ряд, 3×2 или 2×3 — никогда 4+2.
    expect(balancedColumns(6, 1100, 168, 8)).toBe(6);
    expect(balancedColumns(6, 720, 168, 8)).toBe(3); // помещается 4 → 3×2
    expect(balancedColumns(6, 520, 168, 8)).toBe(3);
    expect(balancedColumns(6, 400, 168, 8)).toBe(2);
    expect(balancedColumns(6, 150, 168, 8)).toBe(1);
    expect(balancedColumns(1, 900, 168, 8)).toBe(1);
    expect(balancedColumns(7, 720, 168, 8)).toBe(4); // простое — сколько помещается
  });
});
