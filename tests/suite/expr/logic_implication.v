// Logical implication and equivalence (IEEE 1800-2017 11.4.7): `a -> b` is `!a || b`,
// `a <-> b` is `(a -> b) && (b -> a)`, with x for unknown operands.
module test;
  reg a, b;
  reg [3:0] v;
  wire w = a -> b;
  integer fails = 0;

  task check(input reg got, input reg want, input integer line);
    if (got !== want) begin
      $display("FAILED line %0d: got %b, want %b", line, got, want);
      fails++;
    end
  endtask

  initial begin
    a = 0; b = 0; #1 check(w, 1, `__LINE__); check(a <-> b, 1, `__LINE__);
    a = 0; b = 1; #1 check(w, 1, `__LINE__); check(a <-> b, 0, `__LINE__);
    a = 1; b = 0; #1 check(w, 0, `__LINE__); check(a <-> b, 0, `__LINE__);
    a = 1; b = 1; #1 check(w, 1, `__LINE__); check(a <-> b, 1, `__LINE__);
    a = 1'bx; b = 1; #1 check(w, 1, `__LINE__); check(a <-> b, 1'bx, `__LINE__);
    a = 1'bx; b = 0; #1 check(w, 1'bx, `__LINE__);
    a = 0; b = 1'bx; #1 check(w, 1, `__LINE__);
    v = 4'b0100;
    check(v -> 0, 0, `__LINE__);      // a vector operand is true when nonzero
    check(0.0 <-> 0, 1, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
