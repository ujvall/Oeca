import { expect, test } from "vitest";
import { StreamController } from "./streamController";

test("Should add data to stream", async () => {
  const streamController = new StreamController<number>();
  streamController.add(1);
  streamController.add(2);
  streamController.add(3);

  const { asyncIterator } = streamController.createAsyncIterator();

  let emitted = [];

  for await (const value of asyncIterator) {
    emitted.push(value);
    if (value === 3) {
      break;
    }
  }

  expect(emitted).toEqual([1, 2, 3]);
});

test("Should be able to resume after break", async () => {
  const streamController = new StreamController<number>();
  streamController.add(1);
  streamController.add(2);
  streamController.add(3);

  const iterator = streamController.createAsyncIterator();

  let emitted = [];

  for await (const value of iterator.asyncIterator) {
    emitted.push(value);

    if (emitted.length === 2) {
      break;
    }
  }

  iterator.releaseLock();
  expect(emitted).toEqual([1, 2]);
  streamController.add(10);

  for await (const value of streamController.createAsyncIterator()
    .asyncIterator) {
    emitted.push(value);

    if (value === 10) {
      break;
    }
  }

  expect(emitted).toEqual([1, 2, 3, 10]);
});
