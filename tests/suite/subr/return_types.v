// Function results take the width, sign and kind of their declared return
// type, also through a typedef (IEEE 1800-2017 13.4).
module top;
  typedef byte B;
  typedef real R;
  typedef logic [3:0][1:0] P;
  typedef int unsigned U;

  function B fb(input int x); return x; endfunction
  function R fr(input real x); return x / 2; endfunction
  function P fp(input int x); return x; endfunction
  function int unsigned fu(input int x); return x; endfunction
  function U ftu(input int x); return x; endfunction
  function shortint fs(input int x); return x; endfunction
  function bit [5:0] fbit(input int x); return x; endfunction

  module_with_param #(byte) mb();
  module_with_param #(int unsigned) mu();

  int fails = 0;
  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  initial begin
    check($bits(fb(0)) == 8, "typedef byte: 8 bits");
    check(fb(200) < 0, "typedef byte: signed");
    check(fr(3.0) == 1.5, "typedef real: a real");
    check($bits(fp(0)) == 8, "typedef of packed dimensions: whole width");
    check(fp(9'h1ff) == 8'hff, "typedef of packed dimensions: truncated");
    check(fu(-1) > 0, "int unsigned: unsigned");
    check(ftu(-1) > 0, "typedef int unsigned: unsigned");
    check($bits(fs(0)) == 16 && fs(16'h8000) < 0, "shortint: 16 bits, signed");
    check($bits(fbit(0)) == 6, "bit [5:0]: 6 bits");
    #2 if (fails == 0) $display("PASSED");
  end
endmodule

// a return type given by a type parameter
module module_with_param #(type T = int);
  function T id(T x); return x; endfunction
  initial begin
    #1;
    if ($bits(T) == 8) top.check(id(8'h80) < 0 && $bits(id(0)) == 8, "type parameter byte");
    if ($bits(T) == 32) top.check(id(-1) > 0, "type parameter int unsigned");
  end
endmodule
