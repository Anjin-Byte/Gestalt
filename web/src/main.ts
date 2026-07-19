// The entry module: boots the app and nothing else. Every behavior lives in
// app.ts, which imports without side effects so the test suite can reach it.
import { main } from "./app";

void main();
