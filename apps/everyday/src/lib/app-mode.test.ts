import { describe, expect, it } from "vitest";
import { joinInviteUrl, publicJoinOrigin, trpcUrl } from "./app-mode";

describe("app mode URL helpers", () => {
  it("uses a same-origin tRPC endpoint by default", () => {
    expect(trpcUrl()).toBe("/api/trpc");
  });

  it("builds invitation URLs from an explicit HTTPS sync origin", () => {
    expect(joinInviteUrl("a token", { syncUrl: "https://mesh.example/" })).toBe(
      "https://mesh.example/join?token=a%20token",
    );
  });

  it("does not accept executable URL schemes", () => {
    expect(publicJoinOrigin("javascript:alert(1)")).toBe("");
  });
});
