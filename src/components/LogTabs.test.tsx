import { describe, expect, it, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import LogTabs from "./LogTabs";
import { useStore } from "../store/useStore";

function seedOneTab() {
  useStore.setState({
    tabs: [
      {
        id: "t1",
        ns: "default",
        pod: "pod-1",
        containers: ["c1"],
        container: null,
        status: "streaming",
        error: null,
        options: { tail: 500, timestamps: false, since: null },
        lines: ["hello \x1b[31mworld\x1b[0m"],
        autoScroll: true,
      },
    ],
    activeTabId: "t1",
  });
}

describe("LogTabs", () => {
  beforeEach(() => {
    useStore.setState({ tabs: [], activeTabId: null });
    seedOneTab();
  });

  it("render LogTabPanel với dòng log ANSI (regression: element type invalid)", () => {
    render(<LogTabs />);
    expect(screen.getByText(/hello/)).toBeTruthy();
    expect(screen.getByText(/world/)).toBeTruthy();
  });

  it("phóng to / thu nhỏ khung log qua nút trên tab bar", async () => {
    const user = userEvent.setup();
    render(<LogTabs />);

    const btn = screen.getByRole("button", { name: "Phóng to khung log" });
    await user.click(btn);
    expect(screen.getByRole("button", { name: "Thu nhỏ khung log" })).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Thu nhỏ khung log" }));
    expect(screen.getByRole("button", { name: "Phóng to khung log" })).toBeTruthy();
  });
});
