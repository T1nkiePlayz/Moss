import { describe, expect, it } from "vitest";
import { createMossClient, streamEvents } from "../src/index.js";

describe("createMossClient", () => {
  it("sends the bearer token to /api/v1/servers and types the response", async () => {
    const seen: Request[] = [];
    const fetchStub = async (req: Request) => {
      seen.push(req);
      return new Response(JSON.stringify([{ id: "s1", name: "Survival" }]), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    };
    const client = createMossClient({ baseUrl: "http://moss.test/", token: "moss_pat_abc", fetch: fetchStub });
    const { data, error } = await client.GET("/servers");
    expect(error).toBeUndefined();
    // Typecheck-level: data is Server[] | undefined, so these property accesses must compile.
    expect(data?.[0].id).toBe("s1");
    expect(seen[0].url).toBe("http://moss.test/api/v1/servers");
    expect(seen[0].headers.get("authorization")).toBe("Bearer moss_pat_abc");
  });

  it("fills path parameters", async () => {
    let url = "";
    const client = createMossClient({
      baseUrl: "http://moss.test/api/v1",
      token: "t",
      fetch: async (req: Request) => {
        url = req.url;
        return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
      },
    });
    await client.POST("/servers/{id}/actions/{action}", { params: { path: { id: "abc", action: "start" } } });
    expect(url).toBe("http://moss.test/api/v1/servers/abc/actions/start");
  });
});

describe("streamEvents", () => {
  it("gets a ticket and parses events", async () => {
    const urls: string[] = [];
    const f = (async (input: string | URL | Request) => {
      const u = String(input);
      urls.push(u);
      if (u.endsWith("/ticket")) return new Response(JSON.stringify({ ticket: "tk" }));
      return new Response(': keep\n\nevent: hello\ndata: {}\n\nevent: server\nid: 3\ndata: {"id":"s1"}\n\n');
    }) as typeof fetch;
    const out = [];
    for await (const e of streamEvents({ baseUrl: "http://moss.test", token: "t", fetch: f })) out.push(e);
    expect(urls[1]).toBe("http://moss.test/api/v1/events?ticket=tk");
    expect(out).toEqual([
      { event: "hello", data: {}, id: undefined },
      { event: "server", data: { id: "s1" }, id: "3" },
    ]);
  });
});
