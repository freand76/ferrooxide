// expected
// A parameter with a string literal value prints as text in $display (ICARUS_NOTES 14).
module m #(parameter P = "abc") ();
  initial $display(P, "|", P + 0);
endmodule
module top;
  parameter A = "PASSED";
  parameter [47:0] B = "PASSED";
  localparam C = "hi";
  parameter D = A;
  parameter E = {"a", "b"};
  m u1();
  m #("xyz") u2();
  m #(65) u3();
  initial begin
    $display(A);
    $display(B);
    $display(C, " ", D, " ", E);
    $display("%s %d", A, C);
    $write(A, "\n");
  end
endmodule
