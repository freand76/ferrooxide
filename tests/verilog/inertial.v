// Inertial delays on continuous assignments and gates; rise/fall/turn-off delays
module top;
  reg b, en;
  reg [3:0] v;
  wire a, g, rf, t;
  wire [3:0] vd;
  assign #10 a = b;
  buf #10 (g, b);
  assign #(3, 7) rf = b;
  bufif1 #(2, 4, 6) (t, b, en);
  assign #5 vd = v;
  initial begin
    $monitor("%0t a=%b g=%b rf=%b t=%b vd=%h b=%b en=%b", $time, a, g, rf, t, vd, b, en);
    b = 0; en = 1; v = 0;
    #20 b = 1;
    #5 b = 0;
    #20 b = 1; v = 4'h3;
    #2 v = 4'h0;
    #12 b = 0; en = 0;
    #8 en = 1; v = 4'hx;
    #1 b = 1'bx;
    #30 $finish;
  end
endmodule
