// A typedef in a block (IEEE 1800-2017 6.18: a data declaration of the block): the type
// is known in the rest of the block only; another block may declare the same name.
module t;
  integer errors = 0;
  typedef enum { A, B, C } e_t;
  initial begin
    for (int i = 0; i < 2; i++) begin
      typedef bit [$bits(e_t) - 1:0] raw_t;
      raw_t r;
      r = '1;
      if ($bits(r) != 32 || r != 32'hffffffff) begin
        $display("FAILED %0d %h", $bits(r), r);
        errors = errors + 1;
      end
    end
    begin
      typedef logic [3:0] raw_t;
      raw_t r = 4'hz;
      if ($bits(r) != 4 || r !== 4'hz) begin
        $display("FAILED %0d %b", $bits(r), r);
        errors = errors + 1;
      end
    end
    if (errors == 0) $display("PASSED");
  end
endmodule
