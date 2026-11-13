// iverilog: -g2012 -gspecify
// specify path delays: the delay of the path whose source changed last; rise/fall delays
module dut (input EN, input I, input a, output O, output q);
  assign O = EN ? I : 1'bz;
  assign q = ~a;
  specify
    specparam tr = 3, tf = 5;
    (I => O) = (2);
    (EN *> O) = (4);
    (a *> q) = (tr, tf);
  endspecify
endmodule

module top;
  reg EN, I;
  reg a;
  tri0 O;
  wire q;
  dut d (EN, I, a, O, q);
  initial begin
    $monitor("%0t EN=%b I=%b O=%b a=%b q=%b", $time, EN, I, O, a, q);
    EN = 0; I = 0; a = 0;
    #5 I = 1;
    #1 EN = 1;
    #10 I = 0;
    #10 a = 1;
    #10 a = 0;
    #10 $finish;
  end
endmodule
