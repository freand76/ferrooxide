module tb;
  reg [3:0] x;
  reg [7:0] mem [0:15];
  integer k;
  function [7:0] sq;
    input [3:0] v;
    begin
      sq = v * v;
    end
  endfunction
  function integer fact(input integer n);
    integer j;
    begin
      fact = 1;
      for (j = 2; j <= n; j = j + 1) fact = fact * j;
    end
  endfunction
  task show(input [7:0] v);
    $display("show %d", v);
  endtask
  localparam F5 = fact(5);
  initial begin
    for (k = 0; k < 16; k = k + 1) mem[k] = sq(k);
    for (k = 0; k < 16; k = k + 3) $display("mem[%0d]=%d", k, mem[k]);
    $display("F5=%0d", F5);
    show(mem[7]);
    for (k = 0; k < 6; k = k + 1) begin
      x = k;
      case (x)
        0: $display("zero");
        1, 2: $display("one or two");
        4'b0011: $display("three");
        default: $display("other %d", x);
      endcase
      casez (x)
        4'b01??: $display("z: 01xx");
        default: $display("z: default");
      endcase
    end
    x = 4'b1z00;
    casex (x)
      4'b1000: $display("casex hit");
      default: $display("casex miss");
    endcase
    $display("%h", mem[20]);
  end
endmodule
