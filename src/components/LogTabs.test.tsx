import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import LogTabs from "./LogTabs";
import { useStore } from "../store/useStore";

describe("LogTabs", () => {
  it("render LogTabPanel với dòng log ANSI (regression: element type invalid)", () => {
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
    render(<LogTabs />);
    expect(screen.getByText(/hello/)).toBeTruthy();
    expect(screen.getByText(/world/)).toBeTruthy();
  });
});
