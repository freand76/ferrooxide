// exit: 1
// a virtual interface handle refers to instances of its own interface only (IEEE 1800-2017 25.9)
interface a_if; logic x; endinterface
interface b_if; logic x; endinterface
module top;
  a_if a();
  b_if b();
  virtual a_if va;
  initial va = b;
endmodule
