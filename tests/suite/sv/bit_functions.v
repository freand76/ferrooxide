// $countones, $onehot, $onehot0, $isunknown (IEEE 1800 20.9): x and z bits are not ones
module t;
  integer errors = 0;
  reg [7:0] a;
  reg [99:0] w;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 8'b0010_1100;
    check(`__LINE__, $countones(a), 3);
    check(`__LINE__, $onehot(a), 0);
    check(`__LINE__, $onehot0(a), 0);
    a = 8'b0100_0000;
    check(`__LINE__, $onehot(a), 1);
    check(`__LINE__, $onehot0(a), 1);
    a = 0;
    check(`__LINE__, $onehot(a), 0);
    check(`__LINE__, $onehot0(a), 1);
    a = 8'b01x0_z000;
    check(`__LINE__, $countones(a), 1);
    check(`__LINE__, $onehot(a), 1);
    check(`__LINE__, $isunknown(a), 1);
    check(`__LINE__, $isunknown(8'h12), 0);
    w = 0; w[99] = 1; w[64] = 1; w[0] = 1;
    check(`__LINE__, $countones(w), 3);
    check(`__LINE__, $bits($countones(w)), 32);
    check(`__LINE__, $countones(2'b11 + 3'd6), 1);   // self-determined: 3 bits, 3+6 = 1
    if (errors == 0) $display("PASSED");
  end
endmodule
