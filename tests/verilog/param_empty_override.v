// An empty positional parameter override (`#( , 4)`) leaves that parameter at its default;
// the next value still goes to the next parameter.
module sub;
  parameter A = 1;
  parameter B = 32;
  parameter C = 0;
  initial #1 $display("%m: A=%0d B=%0d C=%0d", A, B, C);
endmodule
module top;
  sub #( , 4) u1();
  sub #(7, , 9) u2();
  sub #( , , 5) u3();
  sub #(2) u4();
endmodule
