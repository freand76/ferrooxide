// Inertial delays (continuous assignment, gate, UDP): an input change that gives the value
// already on its way leaves that update as it is; a pulse shorter than the delay is dropped.
primitive my_or(o, a, b);
  output o;
  input a, b;
  table
    0 0 : 0;
    1 ? : 1;
    ? 1 : 1;
  endtable
endprimitive

module top;
  reg in1, in2;
  wire outc, outg, outp;
  assign #40 outc = in1 | in2;
  or #40 g1(outg, in1, in2);
  my_or #40 g2(outp, in1, in2);
  initial begin
    $monitor("%0t c=%b g=%b p=%b", $time, outc, outg, outp);
    in1 = 0; in2 = 0;
    #10 in1 = 1;
    #20 in2 = 1;
    #100 in1 = 0; in2 = 0;
    #10 in1 = 1;
    #10 in1 = 0;
    #100 $finish;
  end
endmodule
