// always_comb runs once at time 0 after the initial and always blocks have started
// (IEEE 1800 9.2.2.2.2): it never sees the starting x of an input set at time 0
module sub(input [1:0] s, output reg [3:0] y);
  always_comb begin
    $display("%0t sub comb s=%b", $time, s);
    y = s + 1;
  end
endmodule
module t;
  reg [1:0] s, k; reg [3:0] y2, l; wire [3:0] y;
  sub u(s, y);
  always_comb y2 = {2'b00, k};       // k never changes: only the time-0 run sets y2
  always_latch if (k == 0) l = 7;
  initial begin
    s = 2; k = 0;
    $display("%0t initial y=%b y2=%b l=%b", $time, y, y2, l);
    #0 $display("%0t #0 y=%b y2=%b l=%b", $time, y, y2, l);
    #1 $display("%0t y=%b y2=%b l=%b", $time, y, y2, l);
    s = 1;
    #1 $display("%0t y=%b", $time, y);
  end
endmodule
