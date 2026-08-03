import type {
  CollectionNode,
  EnvironmentView,
  RequestSummary,
  SavedRequest,
  Tree,
  WorkspaceList,
} from "./api";

interface Seed {
  id: string;
  name: string;
  method: string;
  url: string;
  path: string;
  kind?: SavedRequest["kind"];
  body?: string;
}

const USERS: Seed[] = [
  { id: "m1", name: "List users", method: "GET", url: "{{baseUrl}}/users?page=1", path: "users/list-users.toml" },
  {
    id: "m2",
    name: "Create user",
    method: "POST",
    url: "{{baseUrl}}/users",
    path: "users/create-user.toml",
    body: '{\n  "name": "Ada Lovelace",\n  "email": "ada@example.com"\n}',
  },
  { id: "m3", name: "Update user", method: "PATCH", url: "{{baseUrl}}/users/{{userId}}", path: "users/update-user.toml" },
  { id: "m4", name: "Delete user", method: "DELETE", url: "{{baseUrl}}/users/{{userId}}", path: "users/delete-user.toml" },
];

const ADMIN: Seed[] = [
  { id: "m11", name: "Impersonate", method: "POST", url: "{{baseUrl}}/admin/impersonate", path: "users/admin/impersonate.toml" },
];

const ORDERS: Seed[] = [
  { id: "m5", name: "List orders", method: "GET", url: "{{baseUrl}}/orders", path: "orders/list-orders.toml" },
  { id: "m6", name: "Place order", method: "PUT", url: "{{baseUrl}}/orders/{{orderId}}", path: "orders/place-order.toml" },
];

const ROOT: Seed[] = [
  { id: "m7", name: "Health check", method: "GET", url: "{{baseUrl}}/health", path: "health-check.toml" },
  { id: "m8", name: "Viewer", method: "POST", url: "{{baseUrl}}/graphql", kind: "graphql", path: "viewer.toml" },
];

const PUBLIC: Seed[] = [
  { id: "m9", name: "Greeter.SayHello", method: "POST", url: "http://localhost:50051", kind: "grpc", path: "greeter.toml" },
  { id: "m10", name: "Exchange rates", method: "GET", url: "https://api.example.com/rates", path: "exchange-rates.toml" },
];

const ALL = [...USERS, ...ADMIN, ...ORDERS, ...ROOT, ...PUBLIC];

function summary(seed: Seed): RequestSummary {
  return {
    id: seed.id,
    name: seed.name,
    kind: seed.kind ?? "http",
    method: seed.method,
    path: seed.path,
  };
}

export function mockWorkspaces(): WorkspaceList {
  return {
    items: [
      { id: "ws-1", path: "/Users/dev/Mandalo", name: "Mandalo" },
      { id: "ws-2", path: "/Users/dev/Acme", name: "Acme Platform" },
    ],
    active: "ws-1",
  };
}

export function mockTree(): Tree {
  const acme: CollectionNode = {
    id: "acme",
    slug: "acme-api",
    name: "Acme API",
    folders: [
      {
        name: "Users",
        path: "users",
        folders: [
          {
            name: "Admin",
            path: "users/admin",
            folders: [],
            requests: ADMIN.map(summary),
          },
        ],
        requests: USERS.map(summary),
      },
      {
        name: "Orders",
        path: "orders",
        folders: [],
        requests: ORDERS.map(summary),
      },
    ],
    requests: ROOT.map(summary),
  };
  const playground: CollectionNode = {
    id: "playground",
    slug: "playground",
    name: "Playground",
    folders: [],
    requests: PUBLIC.map(summary),
  };
  return { collections: [acme, playground], skipped: [] };
}

export function mockRequest(path: string): SavedRequest {
  const seed = ALL.find((s) => s.path === path) ?? ROOT[0];
  return {
    id: seed.id,
    name: seed.name,
    kind: seed.kind ?? "http",
    method: seed.method,
    url: seed.url,
    headers: [["Accept", "application/json"]],
    body: seed.body ?? null,
    auth: { type: "none" },
    graphql:
      seed.kind === "graphql"
        ? { query: "query {\n  viewer {\n    name\n  }\n}", variables: "{}" }
        : null,
    grpc: null,
    description: null,
    scripts:
      seed.id === "m1"
        ? {
            pre: null,
            post:
              'pm.test("Body has three users", function () {\n' +
              "  pm.expect(pm.response.json().users.length).to.eql(3);\n" +
              "});\n",
          }
        : { pre: null, post: null },
    tests:
      seed.id === "m1"
        ? [
            { kind: "status", op: "eq", value: 200 },
            { kind: "json", path: "$.total", op: "eq", value: 3 },
            { kind: "header", name: "Content-Type", op: "contains", value: "json" },
            { kind: "duration", op: "lt", value: 100 },
          ]
        : [{ kind: "status", op: "eq", value: 200 }],
    captures:
      seed.id === "m1"
        ? [{ from: "body.$.users[0].id", into: "userId", scope: "persist" }]
        : [],
  };
}

export function mockEnvironments(): EnvironmentView[] {
  return [
    {
      name: "staging",
      vars: {
        baseUrl: {
          secret: false,
          value: "https://staging.acme.dev/api/v2",
          set: true,
        },
        userId: { secret: false, value: "42", set: true },
        token: {
          secret: true,
          value: null,
          hosts: ["staging.acme.dev"],
          set: true,
        },
        adminToken: { secret: true, value: null, hosts: [], set: false },
      },
    },
    {
      name: "production",
      vars: {
        baseUrl: { secret: false, value: "https://api.acme.com/v2", set: true },
      },
    },
  ];
}
