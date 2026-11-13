// force/release and procedural assign/deassign hold their value against other writes.
module tb;
  reg [3:0] r, s;
  wire [3:0] w;
  reg [3:0] drv = 4'h3;
  assign w = drv;
  reg clk = 0;
  always @(posedge clk) r <= r + 1;
  initial begin
    r = 0; s = 5;
    #1 clk = 1; #1 clk = 0;
    $display("r=%h w=%h s=%h", r, w, s);
    force r = 4'ha;
    force w = s;
    #1 clk = 1; #1 clk = 0;
    $display("forced: r=%h w=%h", r, w);
    s = 6; drv = 4'h9;
    #1 $display("forced follows s: r=%h w=%h", r, w);
    release r;
    #1 clk = 1; #1 clk = 0;
    $display("released r: r=%h", r);
    assign s = 4'hc;
    s = 1;
    #1 $display("assigned s=%h", s);
    deassign s;
    s = 2;
    #1 $display("deassigned s=%h", s);
  end
endmodule
