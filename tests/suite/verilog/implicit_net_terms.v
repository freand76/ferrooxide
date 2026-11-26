// implicit nets (IEEE 1800-2017 6.10) for undeclared names in the terminals of a primitive
// instance and in an expression connected to a port
primitive inv (z, a);
  output z;
  input a;
  table
    0 : 1;
    1 : 0;
  endtable
endprimitive

module sub(input [1:0] a, output logic [1:0] seen);
  assign seen = a;
endmodule

module top;
  wire [1:0] seen;
  inv u (w, 1'b0);
  sub s (.a({w, !w}), .seen(seen));
  initial #1 if (w === 1'b1 && seen === 2'b10) $display("PASSED");
  else $display("FAILED %b %b", w, seen);
endmodule
