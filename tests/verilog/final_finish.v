// $finish in a final procedure ends the simulation at once: the rest of that procedure and
// the other final procedures do not run (IEEE 1800-2017 9.2.3).
module a;
  reg [7:0] x = 1;
  initial #5 x = 2;
  final begin
    $display("a final x=%0d", x);
    $finish(0);
    $display("not reached");
  end
endmodule
module b;
  final $display("b final");
endmodule
