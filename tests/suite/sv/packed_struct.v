// packed structs and unions (IEEE 1800 7.2-7.3), assignment patterns (10.9)
module t;
  integer errors = 0;
  typedef struct packed {
    logic [3:0] op;
    logic [7:0] a;
    logic       v;
  } ins_t;
  typedef union packed {
    ins_t       i;
    logic [12:0] raw;
  } word_t;
  ins_t x;
  word_t w;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, $bits(ins_t), 13);
    x = '{4'h3, 8'hab, 1'b1};
    check(`__LINE__, x, 13'h0757);
    check(`__LINE__, x.a, 8'hab);
    x.op = 4'hf;
    x.a[0] = 0;
    check(`__LINE__, x, {4'hf, 8'haa, 1'b1});
    w.raw = 13'h1fff;
    check(`__LINE__, w.i.op, 4'hf);
    w.i.v = 0;
    check(`__LINE__, w.raw, 13'h1ffe);
    x = '{default: 0};
    check(`__LINE__, x, 0);
    x = '{a: 8'h12, default: 1'b1};   // named, then the rest
    check(`__LINE__, x.op, 4'h1);
    check(`__LINE__, x.a, 8'h12);
    if (errors == 0) $display("PASSED");
  end
endmodule
