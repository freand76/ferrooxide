// Initial values (declaration initializers, constant continuous assignments) are visible at
// time 0 and give "any change" events (but no edges) after the processes have started, as in Icarus
module top;
  reg c1 = 1'b1;
  wire c2 = 1'b1;
  reg [3:0] a = 4'd3;
  wire [3:0] w;
  assign w = 4'd9;
  wire [1:0] m;
  assign m = 2'b01;
  assign m = 2'b11;
  wire signed [63:0] in;
  wire signed [2:0] c;
  reg signed [63:0] o;
  assign in = 64'hF333333333333392;
  assign c = 3'sd3;
  always @* o = in % c;
  always @(posedge c1) $display("%0t posedge c1", $time);
  always @(posedge c2) $display("%0t posedge c2", $time);
  always @(c1) $display("%0t any c1", $time);
  always @(a) $display("%0t any a", $time);
  always @(w) $display("%0t any w %0d", $time, w);
  initial begin
    $display("%0t init %b %b %0d %0d %b", $time, c1, c2, a, w, m);
    #0 $display("%0t init #0", $time);
    #1 $display("%0d", o);
  end
endmodule
