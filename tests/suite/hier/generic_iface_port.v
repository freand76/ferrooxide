// expected
// Generic interface ports (IEEE 1800-2017 25.3.3): `interface p` and `interface.mp p` take
// the interface of the instance connected to them, by position or by name.
interface bus_if;
  logic [7:0] data;
  logic valid;
  modport src(output data, valid);
  modport dst(input data, valid);
endinterface
interface other_if;
  int n;
endinterface
module producer(interface.src b);
  initial begin #1 b.data = 8'h5a; b.valid = 1; end
endmodule
module consumer(interface b);
  always @(posedge b.valid) $display("got %h", b.data);
endmodule
module counter(interface c);
  initial #2 $display("n = %0d", c.n);
endmodule
module top;
  bus_if bi();
  other_if oi();
  initial oi.n = 42;
  producer p(bi);
  consumer c(.b(bi));
  counter k(oi);
endmodule
