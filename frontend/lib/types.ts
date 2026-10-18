export type SessionState = "up" | "down" | "unknown";

export interface Totals {
  messages: number;
  routeMonitoring: number;
  statisticsReport: number;
  peerUp: number;
  peerDown: number;
  initiation: number;
  termination: number;
  routeMirroring: number;
  announcements: number;
  withdrawals: number;
  parseErrors: number;
  rejectedConnections: number;
  bytes: number;
}

export interface PeerRef {
  key: string;
  ip: string;
  asn: number;
  bgpId: string;
  session: SessionState;
  postPolicy: boolean;
  adjRibOut: boolean;
  ipv6: boolean;
  asn16bit: boolean;
}

export interface PeerView extends PeerRef {
  peerType: string;
  distinguisher: number;
  sessionSince: number | null;
  firstSeen: number;
  lastSeen: number;
  router: string | null;
  localAddr: string | null;
  localPort: number | null;
  remotePort: number | null;
  messages: number;
  announcements: number;
  withdrawals: number;
  prefixes: number;
  lastDownReason: string | null;
  stats: StatView[];
}

export interface RouteView {
  prefix: string;
  action: "announce" | "withdraw";
  nextHop: string | null;
  asPath: number[];
  asPathStr: string;
  originAsns: number[];
  origin: string | null;
  med: number | null;
  localPref: number | null;
  communities: string[];
  pathId: number | null;
}

export interface RouteEntry {
  prefix: string;
  pathId: number | null;
  peerKey: string;
  peerIp: string;
  peerAsn: number;
  nextHop: string | null;
  asPath: number[];
  asPathStr: string;
  originAsns: number[];
  origin: string | null;
  med: number | null;
  localPref: number | null;
  communities: string[];
  firstSeen: number;
  updatedAt: number;
  updates: number;
}

export interface StatView {
  code: number;
  name: string;
  value: string;
  kind: string;
}

export interface TlvView {
  ty: number;
  name: string;
  value: string;
}

export interface OpenInfo {
  version: number;
  asn: number;
  holdTime: number;
  bgpId: string;
  capabilities: string[];
}

export interface PeerUpView {
  localAddr: string;
  localPort: number;
  remotePort: number;
  sentOpen: OpenInfo | null;
  receivedOpen: OpenInfo | null;
}

export interface MessageView {
  id: number;
  ts: number;
  receivedAt: number;
  msgType: string;
  msgTypeCode: number;
  size: number;
  routerAddr: string;
  routerName: string | null;
  peer: PeerRef | null;
  bgpType: string | null;
  announced: number;
  withdrawn: number;
  routesTotal: number;
  routesTruncated: number;
  routes: RouteView[];
  stats: StatView[];
  peerDownReason: string | null;
  peerDownDetail: string | null;
  peerUp: PeerUpView | null;
  tlvs: TlvView[];
  terminationReason: string | null;
  error: string | null;
  rawHex: string;
  rawTruncated: boolean;
}

export interface BucketView {
  t: number;
  messages: number;
  announcements: number;
  withdrawals: number;
  errors: number;
}

export interface AsnCount {
  asn: number;
  count: number;
}

export interface InstanceStatus {
  id: string;
  name: string;
  routerIp: string;
  description: string;
  enabled: boolean;
  online: boolean;
  activeConnections: number;
  peers: number;
  messages: number;
  bytes: number;
  lastSeen: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface DemoStatus {
  running: boolean;
  startedAt: number | null;
  sent: number;
}

export interface Overview {
  startedAt: number;
  overviewSeq: number;
  uptimeSeconds: number;
  bmpBind: string;
  httpBind: string;
  bmpListening: boolean;
  demo: DemoStatus;
  totals: Totals;
  ratePerSec: number;
  activeConnections: number;
  connectionsTotal: number;
  maxMsgSize: number;
  idleTimeoutSecs: number;
  maxConnections: number;
  peersTotal: number;
  peersUp: number;
  activeRoutes: number;
  allowlistEnforced: boolean;
  instances: InstanceStatus[];
  buckets: BucketView[];
  peers: PeerView[];
  topOrigins: AsnCount[];
  topPeerAsns: AsnCount[];
  lastError: string | null;
}

export interface RoutesResponse {
  total: number;
  returned: number;
  page: number;
  pageSize: number;
  totalPages: number;
  truncated: boolean;
  routes: RouteEntry[];
}

export interface ConnectionView {
  id: number;
  peer: string;
  routerIp: string;
  routerName: string | null;
  since: number;
  lastMessageAt: number;
  messages: number;
  bytes: number;
  state: string;
  closeReason: string | null;
}

export interface ConnectionsResponse {
  active: ConnectionView[];
  recent: ConnectionView[];
}

export interface AdminSessionInfo {
  authenticated: boolean;
  expiresAt: number | null;
  passwordConfigured: boolean;
}

export interface RuntimeConfig {
  maxMessages: number;
  maxRoutes: number;
  maxPeers: number;
  messageResetSecs: number;
}

export interface AdminInstancesResponse {
  instances: InstanceStatus[];
  enforceAllowlist: boolean;
  runtime: RuntimeConfig;
}

export interface InstanceInput {
  name: string;
  routerIp: string;
  description: string;
  enabled: boolean;
}

export type ServerEvent =
  | { type: "overview"; data: Overview }
  | { type: "message"; data: MessageView }
  | { type: "lagged"; data: { skipped: number } };
