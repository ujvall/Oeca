// Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md.
/** oeca: how much may be queued and unread. */
export type Bound<T> = { cost(data: T): number; limit(): number; overflow(e: Error): void };
const CEILING = 2 ** 40;

/**
 * A simple stream controller like in Dart language.
 */
export class StreamController<T> {
  private _stream: ReadableStream<T>;
  private _controller: ReadableStreamDefaultController<T>;
  private _bound?: Bound<T>;

  constructor(bound?: Bound<T>) {
    let controller!: ReadableStreamDefaultController<T>;
    this._bound = bound;
    this._stream = new ReadableStream<T>(
      {
        start(c) {
          controller = c;
        },
      },
      // oeca: the stream counts what is queued; desiredSize says how much that is
      bound && { highWaterMark: CEILING, size: (data?: T) => bound.cost(data as T) },
    );
    this._controller = controller;
  }

  public add(data: T) {
    const bound = this._bound;
    const free = this._controller.desiredSize;
    if (bound && free !== null && CEILING - free + bound.cost(data) > bound.limit()) {
      const e = new Error("The other device sent more than this page will hold");
      this.error(e); // drops what is queued
      bound.overflow(e);
      return;
    }
    try {
      this._controller.enqueue(data);
    } catch {
      // oeca: errored (connection gone): late messages are dropped
    }
  }

  /** oeca: no more data: queued messages are still read, then reads end. */
  public close() {
    try {
      this._controller.close();
    } catch {
      // already closed or errored
    }
  }

  /** oeca: makes pending and future reads reject (the connection is gone). */
  public error(e: Error) {
    try {
      this._controller.error(e);
    } catch {
      // already closed or errored
    }
  }

  public async readNext(): Promise<T> {
    const reader = this._stream.getReader();

    try {
      const { value, done } = await reader.read();
      if (done) {
        throw new Error("No more data");
      }
      return value as T;
    } finally {
      reader.releaseLock();
    }
  }

  public createAsyncIterator() {
    let reader: ReadableStreamDefaultReader<T>;
    const asyncIterator = this._createAsyncIterator((r) => (reader = r));
    return {
      asyncIterator: asyncIterator as AsyncGenerator<T>,
      releaseLock: () => reader.releaseLock(),
    };
  }

  private async *_createAsyncIterator(
    onReader: (reader: ReadableStreamDefaultReader<T>) => void,
  ) {
    const reader = this._stream.getReader();
    onReader(reader);
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        yield value;
      }
    } finally {
      reader.releaseLock();
    }
  }
}
