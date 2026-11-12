module tb;
  reg go = 0;
  reg [7:0] data;
  reg clk = 0;
  always #2 clk = !clk;
  initial begin
    wait (go);
    $display("%0t go seen", $time);
    @(negedge clk) $display("%0t negedge", $time);
    data = 8'd1;
    while (data < 100) data = data * 3;
    $display("data=%d", data);
    begin : blk
      forever begin
        @(posedge clk);
        data = data + 1;
        if (data == 250) disable blk;
      end
    end
    $display("%0t data=%d", $time, data);
    $finish;
  end
  initial #7 go = 1;
endmodule
