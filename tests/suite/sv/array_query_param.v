// Array query functions of parameters, expressions and dynamic arrays (IEEE 1800-2017 20.7):
// one packed dimension, the declared range, or [width-1:0]; [0:size-1] for a queue; $countbits counts the bits equal to any of
// the control values (20.9).
module test;
  localparam [3:10] P = 0;
  localparam Q = 16'hffff;
  reg [3:0] v = 4'b01zx;
  reg c = 1'bz;
  bit [0:6] d[];
  int q[$];
  integer fails = 0;

  task check(input integer got, input integer want, input integer line);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d, want %0d", line, got, want);
      fails++;
    end
  endtask

  initial begin
    check($left(P), 3, `__LINE__);
    check($right(P), 10, `__LINE__);
    check($low(P), 3, `__LINE__);
    check($high(P), 10, `__LINE__);
    check($size(P), 8, `__LINE__);
    check($increment(P), -1, `__LINE__);
    check($left(Q), 15, `__LINE__);
    check($size(Q + 1'b1), 16, `__LINE__);
    check($dimensions(Q), 1, `__LINE__);
    check($countbits(v, 1'b1), 1, `__LINE__);
    check($countbits(v, 1'bx, c), 2, `__LINE__);
    check($countbits(v, '0, '1, 'x, 'z), 4, `__LINE__);
    check($countbits(70'b0, 1'b0), 70, `__LINE__);
    d = new[3];
    q = '{5, 6};
    check($size(d), 3, `__LINE__);
    check($size(d[1]), 7, `__LINE__);
    check($right(d), 2, `__LINE__);
    check($high(q), 1, `__LINE__);
    check($left(q), 0, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
