// expected
// Sequence operators (16.9): `and` ends with the later operand, `intersect` needs equal
// lengths, `throughout`, `within`, ranged repetition; property `not` and `if` (16.12).
module t;
  reg clk = 0;
  reg a = 0, b = 0, c = 0;
  //                 k: 9876543210
  reg [9:0] A = 10'b0001110111;
  reg [9:0] B = 10'b0010101010;
  reg [9:0] C = 10'b0000110001;
  integer i;
  always #5 clk = ~clk;
  cover sequence (@(posedge clk) (a ##1 b) and (c ##2 1)) $display("%0t and", $time);
  cover sequence (@(posedge clk) (a ##1 a) intersect (1 ##1 b)) $display("%0t intersect", $time);
  cover sequence (@(posedge clk) a throughout (b[->1])) $display("%0t throughout", $time);
  cover sequence (@(posedge clk) b within (a[*3])) $display("%0t within", $time);
  cover sequence (@(posedge clk) c ##1 a[*1:3] ##1 !a) $display("%0t rep", $time);
  assert property (@(posedge clk) not (a ##1 !a)) else $display("%0t not", $time);
  assert property (@(posedge clk) if (c) a else !b) else $display("%0t if", $time);
  initial begin
    for (i = 0; i < 10; i = i + 1) begin
      a = A[i]; b = B[i]; c = C[i];
      @(negedge clk);
    end
    a = 0; b = 0; c = 0;
    #30 $finish;
  end
endmodule
