import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import Modal from "./Modal";

describe("Modal", () => {
  it("render title và children", () => {
    render(
      <Modal title="Quản lý profile">
        <p>nội dung thân</p>
      </Modal>,
    );

    expect(
      screen.getByRole("heading", { name: "Quản lý profile" }),
    ).toBeInTheDocument();
    expect(screen.getByText("nội dung thân")).toBeInTheDocument();
  });

  it("click nút đóng gọi onClose", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(
      <Modal title="T" onClose={onClose}>
        <span>x</span>
      </Modal>,
    );

    await user.click(screen.getByRole("button", { name: "Đóng" }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("nhấn Escape gọi onClose", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(
      <Modal title="T" onClose={onClose}>
        <span>x</span>
      </Modal>,
    );

    await user.keyboard("{Escape}");

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("không có onClose → không render nút đóng", () => {
    render(
      <Modal title="T">
        <span>x</span>
      </Modal>,
    );

    expect(screen.queryByRole("button", { name: "Đóng" })).toBeNull();
  });
});
